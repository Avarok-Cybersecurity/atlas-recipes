// SPDX-License-Identifier: AGPL-3.0-only

//! Three real agents in one process, over localhost TLS.
//!
//! Each agent gets its own loopback IP (`127.0.0.x` — a fleet shares ONE
//! peer port, so the agents are told apart by address exactly as real
//! machines are), its own on-disk identity and pin store, a real
//! `LocalFleet`, and the production serving path (`serve_peer_connection`).
//! Only the container runtime is faked. The suites in `relay_tests.rs` and
//! `relay_grant_tests.rs` drive the real `ControlDriver` through this.

use super::peer_serve::{PeerServe, serve_peer_connection};
use crate::control::ControlHost;
use crate::fleet::LocalFleet;
use crate::identity::{Identity, PinStore};
use crate::launcher::{Launcher, RecordingLauncher};
use crate::peer::link::{SelfIntro, query};
use crate::peer::tls::{PinnedPeerVerifier, server_config};
use crate::peer::wire::{PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use atlasctl_core::registry::RegistrySet;
use atlasctl_protocol::fleet::{
    DisplayName, Launchability, LinkClass, NodeAddress, NodeId, VouchedPeer,
};
use atlasctl_protocol::msg::{ControlRep, ControlReq};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) struct Tmp(pub PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-relay-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&p).expect("scratch");
        Self(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) struct TestAgent {
    pub tmp: Tmp,
    pub identity: Arc<Identity>,
    pub pins: PinStore,
    pub fleet: Arc<LocalFleet>,
    pub launcher: Arc<RecordingLauncher>,
    pub ip: IpAddr,
    /// The fleet-wide peer port, once serving has bound it.
    pub port: u16,
    /// TCP connections accepted, so a test can assert "never dialled".
    pub accepted: Arc<AtomicUsize>,
}

impl TestAgent {
    pub fn id(&self) -> NodeId {
        self.identity.id()
    }

    pub fn sock(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    /// The raw pin file, for asserting a digest can never write a pin.
    pub fn pin_file_bytes(&self) -> Vec<u8> {
        std::fs::read(self.tmp.0.join("peers.json")).unwrap_or_default()
    }
}

/// A fresh agent at `ip`, not yet serving.
pub(super) fn agent(tag: &str, name: &str, ip: &str) -> TestAgent {
    let tmp = Tmp::new(tag);
    let identity = Arc::new(Identity::load_or_create(&tmp.0).expect("identity"));
    let pins = PinStore::new(&tmp.0);
    let ip: IpAddr = ip.parse().expect("ip");
    let fleet = Arc::new(LocalFleet::new(
        Identity::load_or_create(&tmp.0).expect("same key from disk"),
        pins.clone(),
        DisplayName::new(name),
        vec![NodeAddress {
            iface: "lo".to_owned(),
            addr: ip.to_string(),
            class: LinkClass::Ethernet,
            speed_mbps: Some(1_000),
            prefix_len: 8,
            rdma: false,
        }],
        Launchability::yes(),
        "GB10".to_owned(),
    ));
    TestAgent {
        tmp,
        identity,
        pins,
        fleet,
        launcher: Arc::new(RecordingLauncher::new()),
        ip,
        port: 0,
        accepted: Arc::new(AtomicUsize::new(0)),
    }
}

/// Serve the agent's peer channel at `(its ip, port)` — `0` picks the
/// fleet-wide port — through the production dispatch. Returns the bound port.
pub(super) async fn spawn_serving(
    a: &mut TestAgent,
    port: u16,
    answer_budget: Duration,
    launcher: Arc<dyn Launcher>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind((a.ip, port))
        .await
        .expect("bind");
    a.port = listener.local_addr().expect("addr").port();
    let cfg = server_config(
        &a.identity,
        PinnedPeerVerifier::pinned(a.pins.clone(), None),
    )
    .expect("tls config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let serve = Arc::new(PeerServe {
        identity: Arc::clone(&a.identity),
        pins: a.pins.clone(),
        fleet: Arc::clone(&a.fleet),
        rank: Arc::new(NoRank),
        control: Arc::new(ControlHost::new(
            RegistrySet::builtin_only(),
            launcher,
            None,
            Ok(()),
            "NVIDIA GB10".to_owned(),
        )),
        peer_port: a.port,
        answer_budget,
    });
    let accepted = Arc::clone(&a.accepted);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            accepted.fetch_add(1, Ordering::SeqCst);
            let acceptor = acceptor.clone();
            let serve = Arc::clone(&serve);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let Some(peer) = super::peer_of(&tls) else {
                    return;
                };
                if !serve.pins.is_pinned(peer).unwrap_or(false) {
                    return;
                }
                serve_peer_connection(&mut tls, &serve, peer).await;
            });
        }
    });
    a.port
}

/// Write `peer`'s pin into `holder`'s store, exactly as a ceremony would.
pub(super) fn pin(holder: &TestAgent, peer: &TestAgent, granted: bool) {
    crate::fleet::record_pairing(
        &holder.pins,
        peer.id(),
        &hex::encode(peer.identity.public().as_bytes()),
        DisplayName::new("peer"),
        0,
        Some(peer.ip.to_string()),
        false,
    )
    .expect("pin");
    if granted {
        assert!(holder.pins.set_controller(peer.id(), true).expect("grant"));
    }
}

/// One tick of the production poll loop: query the peer, record its report
/// and — when it sent one — its digest. Returns the digest for assertions.
pub(super) async fn poll(from: &TestAgent, to: &TestAgent) -> Option<Vec<VouchedPeer>> {
    let report = query(
        &from.identity,
        from.pins.clone(),
        to.sock(),
        to.id(),
        from.fleet.classify_peer_address(&to.ip.to_string()),
        &SelfIntro::new(from.fleet.can_launch(), "GB10"),
        &from.fleet.local_addresses(),
    )
    .await
    .expect("poll");
    let digest = report.vouched.clone();
    if let Some(d) = report.vouched.clone() {
        from.fleet.record_vouches(to.id(), d);
    }
    from.fleet.record_report(report);
    digest
}

/// A second-hand claim, as a (possibly lying) voucher would state it.
pub(super) fn claim(node: NodeId, addresses: Vec<NodeAddress>, reachable: bool) -> VouchedPeer {
    VouchedPeer {
        node,
        name: DisplayName::new("claimed"),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        addresses,
        link: LinkClass::Ethernet,
        reachable,
        vitals: None,
        vitals_age_s: None,
    }
}

/// The production origin-side driver, on this test's runtime.
pub(super) fn driver(a: &TestAgent, port: u16) -> crate::peer::control::ControlDriver {
    crate::peer::control::ControlDriver::new(
        Arc::clone(&a.identity),
        a.pins.clone(),
        Arc::clone(&a.fleet),
        port,
        tokio::runtime::Handle::current(),
    )
}

/// A TCP listener that only counts, for proving an address is never dialled.
pub(super) async fn counting_listener(ip: &str, port: u16) -> Arc<AtomicUsize> {
    let listener = tokio::net::TcpListener::bind((ip.parse::<IpAddr>().expect("ip"), port))
        .await
        .expect("bind hostile listener");
    let count = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&count);
    tokio::spawn(async move {
        while let Ok((_tcp, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
        }
    });
    count
}

/// An instrumented peer: real TLS under the given identity and pin store,
/// but a hand-rolled frame loop that RECORDS everything after the hello.
///
/// `version_max` is what its hello advertises (so it can play an old build),
/// and `delay` is how long it sits on a `Control` before answering — the
/// budget suite's slow target.
pub(super) async fn spawn_fake_peer(
    a: &TestAgent,
    port: u16,
    version_max: Option<u32>,
    delay: Duration,
) -> (u16, Arc<Mutex<Vec<PeerFrame>>>) {
    let listener = tokio::net::TcpListener::bind((a.ip, port))
        .await
        .expect("bind");
    let bound = listener.local_addr().expect("addr").port();
    let cfg = server_config(
        &a.identity,
        PinnedPeerVerifier::pinned(a.pins.clone(), None),
    )
    .expect("tls config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&frames);
    let local = a.id();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let Ok(PeerFrame::Hello { .. }) = read_frame(&mut tls).await else {
                    return;
                };
                let hello = PeerFrame::Hello {
                    version: PEER_PROTOCOL_VERSION,
                    name: "fake-peer".to_owned(),
                    can_launch: true,
                    accelerator: String::new(),
                    os: "Linux".to_owned(),
                    addresses: Vec::new(),
                    version_max,
                    vouched: None,
                };
                if write_frame(&mut tls, &hello).await.is_err() {
                    return;
                }
                while let Ok(frame) = read_frame(&mut tls).await {
                    log.lock().expect("lock").push(frame.clone());
                    let reply = match frame {
                        PeerFrame::Control { .. } => {
                            tokio::time::sleep(delay).await;
                            PeerFrame::ControlReply {
                                rep: ControlRep::Status {
                                    running: Vec::new(),
                                },
                            }
                        }
                        // A forward reaching a terminal peer would be the R6
                        // violation itself; answer with a loud refusal so
                        // the test that hits it fails visibly.
                        PeerFrame::ControlTo { node, .. } => PeerFrame::ControlReply {
                            rep: ControlRep::Refused {
                                by: local,
                                error: atlasctl_protocol::msg::AgentError::RelayRefused {
                                    node,
                                    via: Some(local),
                                    detail: "a terminal peer received a forward".to_owned(),
                                },
                            },
                        },
                        _ => return,
                    };
                    if write_frame(&mut tls, &reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (bound, frames)
}

/// A rank service the control suites must never touch.
struct NoRank;

impl crate::rank::RankService for NoRank {
    fn render(&self, _: &crate::cluster::RankAssignment) -> anyhow::Result<(String, Vec<String>)> {
        panic!("a control test must not reach the rank service");
    }
    fn content_hash(&self, _: &str) -> anyhow::Result<String> {
        panic!("a control test must not reach the rank service");
    }
    fn recipe_port(&self, _: &str) -> anyhow::Result<Option<u16>> {
        panic!("a control test must not reach the rank service");
    }
    fn prepare(&self, _: &str, _: &crate::cluster::RankAssignment) -> crate::cluster::PrepareReply {
        panic!("a control test must not reach the rank service");
    }
    fn commit(&self, _: &str) -> anyhow::Result<String> {
        panic!("a control test must not reach the rank service");
    }
    fn alive(&self, _: &str) -> anyhow::Result<bool> {
        panic!("a control test must not reach the rank service");
    }
    fn stop(&self, _: &str) -> anyhow::Result<()> {
        panic!("a control test must not reach the rank service");
    }
    fn abort(&self, _: &str) {
        panic!("a control test must not reach the rank service");
    }
}

/// The recipe every suite launches.
pub(super) fn recipe() -> atlasctl_protocol::RecipeId {
    atlasctl_protocol::RecipeId::parse("qwen3.6-27b-fp8").expect("valid id")
}

/// Shorthand: a status request.
pub(super) fn status() -> ControlReq {
    ControlReq::Status
}
