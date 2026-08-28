// SPDX-License-Identifier: AGPL-3.0-only

//! `agent run`, `agent token`, `agent status`.

use crate::cli::AgentRunArgs;
use crate::hostinfo;
use anyhow::{Context, Result};
use atlasctl_agent::launcher::DockerLauncher;
use atlasctl_agent::server::AgentState;
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

    fn pair_at(
        &self,
        target: &str,
        code: &str,
    ) -> anyhow::Result<atlasctl_agent::fleet::PairOutcome> {
        self.0.pair_at(target, code)
    }

    fn trust(
        &self,
        outcome: &atlasctl_agent::fleet::PairOutcome,
        allow_control: bool,
    ) -> anyhow::Result<()> {
        self.0.trust(outcome, allow_control)
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
    // Chosen at compile time so the selection policy above the provider stays
    // one shared path. On macOS the Linux provider found no /sys/class/net and
    // enumerated zero interfaces, so a MacBook advertised no address, was
    // undiscoverable, and minted join invitations with an empty command bar.
    #[cfg(target_os = "macos")]
    let fabric = atlasctl_agent::fabric::macos::MacFabric::new();
    #[cfg(not(target_os = "macos"))]
    let fabric = atlasctl_agent::fabric::linux::LinuxFabric::new();
    // NOT `unwrap_or_default()`. `doctor` learned this already: an
    // enumeration that FAILED is not a machine with no addresses, and the
    // line below makes a claim about the hardware ("no usable network link")
    // that would then be a guess. Keep the two apart — the agent still
    // starts either way, because it can serve this machine's own browser
    // without a cluster link, but it must not say which situation it is in
    // unless it knows.
    let enumerated = fabric.addresses();
    let addresses = enumerated
        .as_ref()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .to_vec();
    let launchability = match &can_launch {
        Ok(()) => atlasctl_protocol::fleet::Launchability::yes(),
        Err(why) => atlasctl_protocol::fleet::Launchability::no(why.clone()),
    };
    eprintln!("node identity: {}", identity.id().short());
    eprintln!(
        "{}",
        link_line(
            enumerated
                .as_ref()
                .map(|a| a.first().map(|f| (f.addr.to_string(), f.class.label())))
                .map_err(|e| format!("{e:#}"))
        )
    );
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

    // Asked once at startup rather than sampled: a machine does not change
    // what accelerator it has while the agent is running, and the fleet view
    // showed an empty string here for every node while the reading it came
    // from already said "NVIDIA GB10".
    let accelerator =
        atlasctl_agent::telemetry::accelerator_name(runner.as_ref()).unwrap_or_default();

    let pins = atlasctl_agent::identity::PinStore::new(&config_dir);
    // Shared by the listener, which admits a stranger only while it is open,
    // and by the browser verb that opens it. One window, or the gate and the
    // invitation would be talking about different things.
    let joining = Arc::new(atlasctl_agent::joining::JoinWindow::default());
    let fleet = atlasctl_agent::fleet::LocalFleet::new(
        atlasctl_agent::identity::Identity::load_or_create(&config_dir)?,
        pins.clone(),
        atlasctl_agent::discovery::local_display_name(),
        addresses.clone(),
        launchability,
        accelerator.clone(),
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
    // The Receiver is dropped, deliberately. Holding it for the process
    // lifetime made `events.receiver_count()` permanently non-zero, which
    // killed the "nobody is watching, do not spawn a process to find out"
    // guard in the vitals loop: the agent shelled out to `docker ps` and
    // sampled the GPU every second forever, including under `--no-browser`
    // where nothing can ever subscribe. Every `send` on this channel already
    // ignores its error, so there is nothing to keep alive.
    let (events, _) = tokio::sync::broadcast::channel(256);

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
            atlasctl_agent::peer::link::SelfIntro::new(can_launch.is_ok(), &accelerator),
        )),
        atlasctl_agent::peer::DEFAULT_PEER_PORT,
    ));

    // The peer channel's control core. Its own instances of the same
    // stateless launcher and telemetry the browser state holds, because the
    // listener outlives any session and cannot borrow from `AgentState` —
    // the checks are identical because both are the one `LocalControl`.
    let control_host = Arc::new(atlasctl_agent::control::ControlHost::new(
        crate::commands::registry_set()?,
        Box::new(DockerLauncher::new(
            Arc::clone(&runner),
            hostinfo::snapshot()?,
            &ROOTLESS_V1,
            Box::new(NvidiaDevices),
        )),
        Some(Box::new(crate::launchtelemetry::LocalLaunchTelemetry::new(
            Arc::clone(&runner),
            atlasctl_agent::launchstats::LaunchSampler::new(Box::new(
                crate::httpscrape::HttpScraper,
            )),
        ))),
        can_launch.clone(),
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
        joining: Some(Arc::clone(&joining)),
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
        relay: Some(Arc::new(atlasctl_agent::peer::control::ControlDriver::new(
            Arc::clone(&identity),
            atlasctl_agent::identity::PinStore::new(&config_dir),
            Arc::clone(&fleet),
            atlasctl_agent::peer::DEFAULT_PEER_PORT,
            rt.handle().clone(),
        ))),
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

    // Claim the port BEFORE announcing it. Everything below — the address, the
    // docker status, the pairing token — is a promise about an agent that is
    // about to exist, and on a port conflict it was all printed and then
    // contradicted. The operator was handed a token for nothing.
    let listener = if args.no_browser {
        None
    } else {
        Some(rt.block_on(atlasctl_agent::server::bind(args.port))?)
    };

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
        atlasctl_agent::daemon::spawn_peer_work(atlasctl_agent::daemon::PeerWork {
            fleet: Arc::clone(&fleet),
            identity: Arc::clone(&identity),
            pins,
            events: events.clone(),
            peer_port: atlasctl_agent::peer::DEFAULT_PEER_PORT,
            rank: Arc::clone(&renderer),
            joining: Arc::clone(&joining),
            accelerator: accelerator.clone(),
            control: control_host,
        });

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
                accelerator: accelerator.clone(),
            },
        );

        let Some(listener) = listener else {
            // Nothing to serve; the peer channel and discovery are the point.
            // Park until signalled rather than returning, which would tear the
            // runtime down and take those with it.
            std::future::pending::<()>().await;
            return Ok(());
        };
        atlasctl_agent::server::serve_on(state, listener).await
    })
}

/// What to tell the operator about this machine's links, as three distinct
/// facts rather than two.
///
/// The distinction is the whole point: "we could not look" and "we looked and
/// there is nothing" send a person to different places, and only one of them
/// is a statement about their network. Pure so it can be tested — the
/// enumeration it describes shells out, which is why the claim it makes is
/// worth pinning down separately from the I/O that feeds it.
fn link_line(first: Result<Option<(String, &'static str)>, String>) -> String {
    match first {
        Err(why) => format!(
            "could not read this machine's network interfaces: {why} \
             — clustering is off until that is fixed; run `atlasctl doctor`"
        ),
        Ok(None) => "no usable network link — this agent cannot take part in a cluster".to_owned(),
        Ok(Some((addr, class))) => format!("cluster address: {addr} ({class})"),
    }
}

#[cfg(test)]
mod link_line_tests {
    use super::link_line;

    #[test]
    fn a_failed_enumeration_is_never_reported_as_an_absent_network() {
        let e = link_line(Err("running `ip -o -4 addr show`: No such file".into()));
        assert!(
            e.contains("could not read"),
            "the operator must learn we could not look: {e}"
        );
        assert!(
            !e.contains("no usable network link"),
            "claiming the network is absent when `ip` is missing sends them to \
             debug hardware that is fine: {e}"
        );
        assert!(
            e.contains("doctor"),
            "and it must say what to run next: {e}"
        );
    }

    #[test]
    fn an_empty_enumeration_still_says_the_network_is_the_problem() {
        // The other side of the same coin: when we DID look and found nothing,
        // hedging would be just as wrong.
        let e = link_line(Ok(None));
        assert_eq!(
            e,
            "no usable network link — this agent cannot take part in a cluster"
        );
    }

    #[test]
    fn a_found_address_is_reported_with_its_link_class() {
        // The class is what tells RoCE from Wi-Fi, which is what the operator
        // needs to know a cluster will actually be fast.
        assert_eq!(
            link_line(Ok(Some(("10.10.10.1".to_owned(), "InfiniBand")))),
            "cluster address: 10.10.10.1 (InfiniBand)"
        );
    }
}
