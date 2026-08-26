// SPDX-License-Identifier: AGPL-3.0-only

//! `agent run`, `agent token`, `agent status`.

use crate::cli::{AgentRunArgs, AgentTokenArgs};
use crate::hostinfo;
use anyhow::{Context, Result, bail};
use atlasctl_agent::launcher::DockerLauncher;
use atlasctl_agent::server::{AgentState, serve};
use atlasctl_agent::token;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use std::sync::Arc;

/// Lets the background loops and the server share one fleet.
///
/// `AgentState` wants an owned `Box<dyn FleetView>` while the daemon loops need
/// an `Arc`; this forwards rather than duplicating the state, so a peer
/// discovered by the loops is visible to the next browser request.
struct FleetHandle(Arc<atlasctl_agent::fleet::LocalFleet>);

impl atlasctl_agent::fleet::FleetView for FleetHandle {
    fn nodes(&self) -> Vec<atlasctl_protocol::fleet::NodeDescriptor> {
        self.0.nodes()
    }

    fn pair(
        &self,
        node: atlasctl_protocol::fleet::NodeId,
        code: &str,
    ) -> anyhow::Result<atlasctl_agent::fleet::PairOutcome> {
        self.0.pair(node, code)
    }

    fn unpair(&self, node: atlasctl_protocol::fleet::NodeId) -> anyhow::Result<bool> {
        self.0.unpair(node)
    }
}

/// Whether this machine can actually run a recipe, and why not if it cannot.
///
/// Probed once at startup and reported to the client, so a browser can say
/// "this box cannot launch" instead of offering a button that will fail. A
/// machine that cannot launch is still useful: it can list and inspect.
fn probe_can_launch(runner: &dyn ProcessRunner) -> Result<(), String> {
    match runner.run(&atlasctl_agent::fleet::docker_probe_argv()) {
        Ok(out) if out.success() => Ok(()),
        Ok(out) => Err(format!(
            "the docker daemon did not answer: {}",
            out.stderr.trim()
        )),
        Err(e) => Err(format!("docker is not available: {e}")),
    }
}

/// Run the agent in the foreground.
pub fn run(args: &AgentRunArgs) -> Result<()> {
    let config_dir = hostinfo::config_dir()?;
    // Checked once, up front, so a permission problem is reported in full
    // rather than as whichever of the three state files happened to be touched
    // first — which is how it surfaced as a bare `Permission denied`.
    crate::configdir::ensure_usable(&config_dir)?;

    // Acquired only when a browser will actually be served. A node that exists
    // to hold a rank talks to its peers over mutually authenticated TLS and
    // never consults this token; making it a startup requirement meant a
    // worker could not run at all because of a credential it would not use.
    let tok = if args.no_browser {
        None
    } else {
        Some(token::load_or_create(&config_dir)?)
    };
    let runner: Arc<dyn ProcessRunner> = Arc::new(StdProcessRunner);
    // In client mode the refusal is not a probe result that could later change
    // its mind — this agent has no business launching anything, and says so.
    let can_launch = if args.client {
        Err(
            "this agent runs in --client mode: it can discover, pair and monitor, \
             but it will not run a model"
                .to_owned(),
        )
    } else {
        probe_can_launch(runner.as_ref())
    };

    // The fleet view is what makes /control show real machines. It is built
    // from this box's own facts — identity, links, launchability — so a fresh
    // agent shows itself correctly before any peer exists.
    let identity = Arc::new(atlasctl_agent::identity::Identity::load_or_create(
        &config_dir,
    )?);
    use atlasctl_agent::fabric::FabricProvider as _;
    let fabric = atlasctl_agent::fabric::linux::LinuxFabric::new();
    let addresses = fabric.addresses().unwrap_or_default();
    let launchability = match &can_launch {
        Ok(()) => atlasctl_protocol::fleet::Launchability::yes(),
        Err(why) => atlasctl_protocol::fleet::Launchability::no(why.clone()),
    };
    eprintln!("node identity: {}", identity.id().short());
    if addresses.is_empty() {
        eprintln!("no usable network link — this agent cannot take part in a cluster");
    } else {
        eprintln!(
            "cluster address: {} ({})",
            addresses[0].addr,
            addresses[0].class.label()
        );
    }
    // Real vitals for this machine. Capabilities are probed once here rather
    // than per sample: on a GB10 that probe is what discovers there is no
    // framebuffer to report, and the answer does not change while we run.
    let vitals = atlasctl_agent::fleet::SystemVitals::new(
        Arc::clone(&runner),
        hostinfo::cache_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")),
    );
    eprintln!(
        "telemetry: gpu={} clock={} memory={}",
        vitals.caps().gpu_util,
        vitals.caps().sm_clock,
        if vitals.caps().unified_memory {
            "unified"
        } else {
            "none"
        }
    );

    let beacon_addrs: Vec<std::net::IpAddr> = addresses
        .iter()
        .filter_map(|a| a.addr.parse().ok())
        .collect();

    // Built before anything that holds a handle to it. Both the cluster
    // previewer and the pairing driver dial another machine, and neither can be
    // constructed without a reactor that already exists.
    let rt = tokio::runtime::Builder::new_multi_thread()
        // Two workers is ample: this serves one local browser, not a fleet.
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting the async runtime")?;

    let pins = atlasctl_agent::identity::PinStore::new(&config_dir);
    let fleet = atlasctl_agent::fleet::LocalFleet::new(
        atlasctl_agent::identity::Identity::load_or_create(&config_dir)?,
        pins.clone(),
        atlasctl_agent::discovery::local_display_name(),
        addresses.clone(),
        launchability,
        String::new(),
    )
    .with_vitals(Box::new(vitals))
    .with_running(Box::new(atlasctl_agent::fleet::DockerRunning(Arc::clone(
        &runner,
    ))))
    // Without this the browser can see peers and not pair with them, which is
    // where the fleet story dead-ended: the dialog existed, the ceremony had
    // nothing to run it.
    .with_pairing(Box::new(crate::peerpairing::RuntimePeerPairing::new(
        Arc::clone(&identity),
        pins.clone(),
        rt.handle().clone(),
    )));

    let fleet = Arc::new(fleet);
    let (events, _keep) = tokio::sync::broadcast::channel(256);

    let renderer: Arc<dyn atlasctl_agent::rank::RankService> =
        Arc::new(crate::rankservice::LocalRankService::new(
            crate::commands::registry_set()?,
            hostinfo::snapshot()?,
            &ROOTLESS_V1,
            Box::new(NvidiaDevices),
            Box::new(atlasctl_core::docker::collective::NcclRoce),
            Arc::clone(&runner),
            crate::rankservice::RankEnvironment {
                can_launch: can_launch.clone(),
                local_addresses: addresses.clone(),
                reachability: Box::new(atlasctl_agent::rendezvous::TcpProbe),
                rdma_devices: atlasctl_agent::fabric::linux::rdma_devices_by_interface(),
            },
        ));

    // Built before the state so the supervisor task can hold the same driver:
    // a rank that dies after commit has to be noticed by something, and the
    // session only exists while a browser is connected.
    let cluster = Arc::new(atlasctl_agent::clusterdriver::ClusterDriver::new(
        Arc::clone(&fleet) as Arc<dyn atlasctl_agent::fleet::FleetView>,
        Arc::clone(&renderer),
        Arc::new(crate::peertransport::PeerTransport::new(
            Arc::clone(&identity),
            pins.clone(),
            rt.handle().clone(),
            atlasctl_agent::peer::link::SelfIntro::new(can_launch.is_ok(), ""),
        )),
        atlasctl_agent::peer::DEFAULT_PEER_PORT,
    ));

    let state = Arc::new(AgentState {
        registry: crate::commands::registry_set()?,
        launcher: Box::new(DockerLauncher::new(
            Arc::clone(&runner),
            hostinfo::snapshot()?,
            &ROOTLESS_V1,
            Box::new(NvidiaDevices),
        )),
        token: tok.clone().unwrap_or_default(),
        can_launch: can_launch.clone(),
        port: args.port,
        allow_dev_origins: args.dev_origins,
        fleet: Some(Box::new(FleetHandle(Arc::clone(&fleet)))),
        telemetry: Some(Box::new(crate::launchtelemetry::LocalLaunchTelemetry::new(
            Arc::clone(&runner),
            atlasctl_agent::launchstats::LaunchSampler::new(Box::new(
                crate::httpscrape::HttpScraper,
            )),
        ))),
        cluster: Some(Arc::clone(&cluster) as Arc<dyn atlasctl_agent::session::ClusterControl>),
        events: events.clone(),
    });

    use atlasctl_agent::session::ClusterControl as _;

    // Watch the cluster stay whole. The settle gate at commit only catches a
    // rank that dies immediately; weights take minutes to load, so a rank that
    // dies during model build passes it and leaves its peers holding GPUs and
    // serving nothing.
    {
        let cluster = Arc::clone(&cluster);
        rt.spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let cluster = Arc::clone(&cluster);
                // Asking a peer dials the network, so it must not run on the
                // async runtime's worker threads.
                let torn = tokio::task::spawn_blocking(move || cluster.supervise()).await;
                if let Ok(Some(why)) = torn {
                    eprintln!("cluster: {why}");
                }
            }
        });
    }

    if args.no_browser {
        // Do not claim a port that was never bound. The whole point of this
        // mode is that there is no browser channel.
        eprintln!("atlasctl agent running (peer channel only, no browser port)");
    } else {
        eprintln!("atlasctl agent listening on 127.0.0.1:{}", args.port);
    }
    // Client mode is a different kind of agent, not a broken one, so it does
    // not report a docker failure it was never going to use — and it does not
    // repeat the docker-group warning, which would be untrue here.
    if args.client {
        eprintln!("mode: control only — this agent will not run a model");
    } else {
        match &can_launch {
            Ok(()) => eprintln!("docker: ok"),
            Err(why) => eprintln!(
                "docker: unavailable — {why}\n  this agent can list and inspect recipes but not launch them"
            ),
        }
    }
    if args.dev_origins {
        eprintln!("accepting development origins — do not leave this on");
    }
    match &tok {
        Some(t) => eprintln!("\npairing token (paste into the website once):\n  {t}\n"),
        None => eprintln!(
            "\nbrowser channel disabled (--no-browser); no pairing token was created.\n\
             This node is reachable by its paired peers over the peer channel.\n"
        ),
    }
    if args.client {
        eprintln!("This agent does not talk to Docker and cannot start a container.");
        eprintln!("It can discover machines, pair with them, and watch what they are doing.");
    } else {
        eprintln!("This agent talks to Docker. On Linux, membership of the `docker` group is");
        eprintln!("root-equivalent, so anything that can drive this agent can do what you can.");
    }
    eprintln!("Stop it with ctrl-c when you are done.\n");

    rt.block_on(async move {
        // Background work: advertise, listen for peers, sample vitals, age out
        // machines that have gone. Started before serving so the first browser to
        // connect already has a populated fleet.
        let discovery: Option<Arc<dyn atlasctl_agent::daemon::DiscoveryPair>> = if args.no_discovery
        {
            eprintln!("discovery disabled; add peers with `atlasctl peer add <host>`");
            None
        } else {
            match atlasctl_agent::discovery::mdns::MdnsDiscovery::new() {
                Ok(d) => Some(Arc::new(d)),
                Err(e) => {
                    eprintln!("discovery unavailable: {e}");
                    None
                }
            }
        };
        // Serving the peer channel is what turns a pairing into a working
        // link: it is how a peer's real vitals and verified link class arrive,
        // rather than a beacon's unauthenticated word for them.
        atlasctl_agent::daemon::spawn_peer_work(
            Arc::clone(&fleet),
            Arc::clone(&identity),
            pins,
            events.clone(),
            atlasctl_agent::peer::DEFAULT_PEER_PORT,
            Arc::clone(&renderer),
        );

        atlasctl_agent::daemon::spawn_all(
            Arc::clone(&fleet),
            events,
            discovery,
            atlasctl_agent::discovery::Beacon {
                id: fleet.id(),
                name: atlasctl_agent::discovery::local_display_name(),
                peer_port: atlasctl_agent::peer::DEFAULT_PEER_PORT,
                addresses: beacon_addrs,
                can_launch: can_launch.is_ok(),
                accelerator: String::new(),
            },
        );

        if args.no_browser {
            // Nothing to serve; the peer channel and discovery are the point.
            // Park until signalled rather than returning, which would tear the
            // runtime down and take those with it.
            std::future::pending::<()>().await;
            return Ok(());
        }
        serve(state, args.port).await
    })
}

/// Print, or rotate, the pairing token.
pub fn token(args: &AgentTokenArgs) -> Result<()> {
    let dir = hostinfo::config_dir()?;
    let tok = if args.rotate {
        let t = token::rotate(&dir)?;
        eprintln!("token rotated — any browser already paired must be given the new one");
        t
    } else {
        token::load_or_create(&dir)?
    };
    println!("{tok}");
    Ok(())
}

/// Report whether an agent is reachable on the default port.
pub fn status() -> Result<()> {
    let addr = format!("127.0.0.1:{}", atlasctl_agent::DEFAULT_PORT);
    match std::net::TcpStream::connect(&addr) {
        Ok(_) => {
            println!("agent: running (listening on {addr})");
            Ok(())
        }
        Err(e) => {
            println!("agent: not running ({e})");
            println!("start it with: atlasctl agent run");
            bail!("no agent is listening on {addr}")
        }
    }
}

/// Print a code for joining this machine to a fleet, and accept one pairing.
///
/// The code is shown HERE and typed on the machine doing the adding. That
/// direction is the entire reason a hostile web page cannot pair anything: it
/// would have to know a code it never saw, on a screen it cannot read.
///
/// # Errors
/// If the identity cannot be loaded or the peer port cannot be bound.
pub fn pair(args: &crate::cli::AgentPairArgs) -> Result<()> {
    use atlasctl_agent::identity::{Identity, PinStore};
    use atlasctl_agent::pairing::{CODE_TTL_SECS, PairingCode};
    use atlasctl_agent::peer::pair::{Role, run};
    use atlasctl_agent::peer::tls::{PinnedPeerVerifier, peer_identity, server_config};
    use atlasctl_protocol::fleet::DisplayName;
    use std::sync::Arc;

    let dir = hostinfo::config_dir()?;
    let identity = Identity::load_or_create(&dir)?;
    let pins = PinStore::new(&dir);
    let code = PairingCode::generate();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;

    // Bind BEFORE printing the code. Printing first and failing after shows
    // someone a code that can never be used — and on a second Spark that
    // already had a pairing waiting, that is exactly what happened.
    let listener = runtime.block_on(async {
        tokio::net::TcpListener::bind(("0.0.0.0", args.port))
            .await
            .with_context(|| {
                format!(
                    "could not bind the peer port {} — is another `atlasctl agent pair` \
                     already waiting?",
                    args.port
                )
            })
    })?;

    println!();
    println!("  Pairing code:  {}", code.grouped());
    println!();
    println!("  On the other machine, run:");
    println!(
        "      atlasctl peer add {} --code {}",
        hostname_hint(),
        code.as_str()
    );
    println!();
    println!("  This code is good for {CODE_TTL_SECS} seconds and for one attempt.");
    println!("  Waiting…");

    let paired = runtime.block_on(async {
        let cfg = server_config(&identity, PinnedPeerVerifier::pairing(pins.clone()))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));

        let accept = async {
            let (tcp, _) = listener.accept().await?;
            let mut tls = acceptor.accept(tcp).await.context("TLS handshake")?;
            let (_, conn) = tls.get_ref();
            let cert = conn
                .peer_certificates()
                .and_then(<[_]>::first)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("the other machine sent no certificate"))?;
            let (peer_id, _) = peer_identity(&cert)?;
            let binding = atlasctl_agent::pairing::binding_from_server(conn)?;
            run(
                &mut tls,
                Role::Responder,
                &identity,
                peer_id,
                code.as_str(),
                binding,
            )
            .await
        };

        // The code expires on its own, so an unattended terminal does not leave
        // a pairing window open indefinitely.
        tokio::time::timeout(std::time::Duration::from_secs(CODE_TTL_SECS), accept)
            .await
            .map_err(|_| anyhow::anyhow!("nobody paired within {CODE_TTL_SECS} seconds"))?
    })?;

    println!();
    println!("  Verification words:  {}", paired.verification);
    println!();
    println!("  The other machine is showing the same words. If it is not,");
    println!("  answer no — something is relaying this connection.");
    println!();
    print!("  Do they match? [y/N] ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("reading confirmation")?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        println!("Nothing was trusted.");
        return Ok(());
    }

    atlasctl_agent::fleet::record_pairing(
        &pins,
        paired.node,
        &paired.public_key,
        DisplayName::new(&paired.name),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        // The responder learns the initiator's address from the connection.
        None,
    )?;
    println!("Paired with {} ({}).", paired.name, paired.node.short());
    Ok(())
}

/// A hostname the other machine can probably reach us on.
///
/// Only ever printed as a hint in a copy-pasteable command — the address that
/// actually matters is whichever one the operator can route to, and they are
/// the ones who know that.
fn hostname_hint() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_else(|_| "<this-machine>".to_owned())
}
