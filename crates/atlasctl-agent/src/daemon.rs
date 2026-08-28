// SPDX-License-Identifier: AGPL-3.0-only

//! The background work an agent does whether or not anyone is looking.
//!
//! Three loops, deliberately independent so one failing does not take the
//! others with it:
//!
//! * **Discovery** advertises this node and records what it hears. A network
//!   that filters multicast is a normal condition, not a fault — the loop says
//!   so once and stops, and `atlasctl peer add` remains a first-class path.
//! * **Vitals** samples this machine and pushes the result to anyone watching.
//!   It is what makes an idle node's clamped clock or full disk visible before
//!   someone launches on it.
//! * **Pruning** ages out sightings so a node that left the network stops being
//!   listed as present, while a *paired* node stays listed as unreachable —
//!   because it is still part of your fleet when it is switched off.
//!
//! Every loop is cancellation-safe and holds only an `Arc`, so shutting the
//! agent down does not need any of them to cooperate.

use crate::discovery::{Advertiser, Beacon, DiscoveryBrowser, DiscoveryEvent};
use crate::fleet::{FleetView, LocalFleet};
use crate::rank::RankService;
use atlasctl_protocol::msg::ServerMsg;
use atlasctl_protocol::msg::fleet::FleetEvent;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// How often vitals are sampled and pushed.
///
/// One second is what a live dashboard wants. Sampling costs a process spawn,
/// so this is deliberately not faster.
pub const VITALS_INTERVAL: Duration = Duration::from_secs(1);

/// How often stale sightings are aged out.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(10);

/// How often paired peers are asked how they are.
///
/// Slower than local vitals on purpose: this opens a TLS connection per peer,
/// and a fleet of idle machines should cost almost nothing to watch.
pub const PEER_POLL_INTERVAL: Duration = Duration::from_secs(5);

// The vitals and prune timers live in `daemon/housekeeping.rs`. They are
// timers over local state; this file is the machine-to-machine half.
mod housekeeping;
mod peer_serve;

#[cfg(test)]
#[path = "daemon/peer_serve_tests.rs"]
mod peer_serve_tests;

#[cfg(test)]
#[path = "daemon/relay_grant_tests.rs"]
mod relay_grant_tests;
#[cfg(test)]
#[path = "daemon/relay_harness.rs"]
mod relay_harness;
#[cfg(test)]
#[path = "daemon/relay_tests.rs"]
mod relay_tests;

use housekeeping::{spawn_prune, spawn_vitals};

/// Start every background loop.
///
/// Returns immediately; the loops run until the process ends.
pub fn spawn_all(
    fleet: Arc<LocalFleet>,
    events: broadcast::Sender<ServerMsg>,
    discovery: Option<Arc<dyn DiscoveryPair>>,
    beacon: Beacon,
) {
    if let Some(d) = discovery {
        spawn_discovery(Arc::clone(&fleet), events.clone(), d, beacon);
    }
    spawn_vitals(Arc::clone(&fleet), events.clone());
    spawn_prune(Arc::clone(&fleet), events);
}

/// Serve the peer channel, and keep paired peers fresh.
///
/// Separate from [`spawn_all`] because it needs this agent's identity, which
/// the fleet view owns privately.
/// `accelerator` is threaded in rather than left blank: `fleet::listing` gives
/// the authenticated peer report PRECEDENCE over the beacon, so an empty string
/// here overwrites a good beacon value and every paired node in the fleet view
/// reads blank. That is the exact symptom `agent run` documents as already
/// fixed — the fix reached the beacon, and this path kept sending "".
pub struct PeerWork {
    /// This machine's view of the fleet.
    pub fleet: Arc<crate::fleet::LocalFleet>,
    /// This node's keypair.
    pub identity: Arc<crate::identity::Identity>,
    /// Who this node trusts.
    pub pins: crate::identity::PinStore,
    /// Where fleet changes are published.
    pub events: broadcast::Sender<ServerMsg>,
    /// The peer channel's port.
    pub peer_port: u16,
    /// What answers rank requests.
    pub rank: Arc<dyn RankService>,
    /// Whether this node is currently accepting a new member.
    pub joining: Arc<crate::joining::JoinWindow>,
    /// This machine's accelerator tag, probed once at startup.
    pub accelerator: String,
    /// The control core a peer's terminal `Control` executes through — the
    /// same seven verbs, the same validation, as this machine's own browser.
    pub control: Arc<crate::control::ControlHost>,
}

pub fn spawn_peer_work(w: PeerWork) {
    spawn_peer_listener(
        Arc::clone(&w.fleet),
        Arc::clone(&w.identity),
        w.pins.clone(),
        w.peer_port,
        w.rank,
        w.joining,
        w.control,
    );
    spawn_peer_poll(
        w.fleet,
        w.identity,
        w.pins,
        w.events,
        w.peer_port,
        w.accelerator,
    );
}

/// Accept connections from paired peers.
fn spawn_peer_listener(
    fleet: Arc<crate::fleet::LocalFleet>,
    identity: Arc<crate::identity::Identity>,
    pins: crate::identity::PinStore,
    port: u16,
    rank: Arc<dyn RankService>,
    joining: Arc<crate::joining::JoinWindow>,
    control: Arc<crate::control::ControlHost>,
) {
    // One serving context for every connection, so the answer budget and the
    // control core cannot differ between two peers of the same agent.
    let serve = Arc::new(peer_serve::PeerServe {
        identity: Arc::clone(&identity),
        pins: pins.clone(),
        fleet: Arc::clone(&fleet),
        rank,
        control,
        peer_port: port,
        answer_budget: crate::peer::control::RELAY_ANSWER_BUDGET,
    });
    tokio::spawn(async move {
        // Pinned peers always; a stranger only while a human has a join code
        // outstanding. Decided during the handshake, so for all the time nobody
        // is onboarding an unpaired agent reaches no further than rustls'
        // ClientHello handling — which was the property `pinned` gave
        // unconditionally, and is the only thing being traded here.
        let gate = {
            let w = Arc::clone(&joining);
            Arc::new(move || w.is_open()) as Arc<dyn Fn() -> bool + Send + Sync>
        };
        let cfg = match crate::peer::tls::server_config(
            &identity,
            crate::peer::tls::PinnedPeerVerifier::while_joining(pins.clone(), gate),
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("peer channel disabled: {e}");
                return;
            }
        };
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "{}",
                    crate::peer::bindfail::peer_bind_failure(port, e.kind(), &e.to_string())
                );
                return;
            }
        };
        eprintln!("peer channel on 0.0.0.0:{port}");

        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                continue;
            };
            let acceptor = acceptor.clone();
            let fleet = Arc::clone(&fleet);
            let joining = Arc::clone(&joining);
            let pins = pins.clone();
            let identity = Arc::clone(&identity);
            let serve = Arc::clone(&serve);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    // An unpaired caller failing the handshake is the system
                    // working, not an incident worth logging.
                    return;
                };

                // Who got in? The verifier admitted either a pinned peer or,
                // inside a join window, a stranger. Those are different
                // conversations and must not share a code path: a stranger may
                // pair and nothing else.
                let peer = peer_of(&tls);
                let pinned = peer.is_some_and(|id| pins.is_pinned(id).unwrap_or(false));
                if !pinned {
                    serve_join(&mut tls, &identity, &pins, &joining, &fleet).await;
                    return;
                }
                let Some(sender) = peer else {
                    return;
                };
                // Introduce ourselves, then answer rank and control frames.
                // Rendering — and any relayed control verb — happens here, on
                // the machine that executes it, from its own vendored recipe.
                peer_serve::serve_peer_connection(&mut tls, &serve, sender).await;
            });
        }
    });
}

/// The identity behind an accepted connection, if it presented one.
fn peer_of<S>(
    tls: &tokio_rustls::server::TlsStream<S>,
) -> Option<atlasctl_protocol::fleet::NodeId> {
    let (_, conn) = tls.get_ref();
    let cert = conn.peer_certificates().and_then(<[_]>::first)?;
    crate::peer::tls::peer_identity(cert).ok().map(|(id, _)| id)
}

/// Serve a machine that is not pinned: it may pair, and nothing else.
///
/// Reached only inside a join window, because that is what let it complete a
/// handshake at all. A failure here is ordinary — a mistyped digit, a dropped
/// connection — and is charged against the window's attempt budget rather than
/// logged as an incident.
async fn serve_join<S>(
    tls: &mut tokio_rustls::server::TlsStream<S>,
    identity: &crate::identity::Identity,
    pins: &crate::identity::PinStore,
    joining: &crate::joining::JoinWindow,
    fleet: &crate::fleet::LocalFleet,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Charges a guess up front. Every path out of this function below is
    // therefore already accounted for, including the two that return without
    // running a ceremony at all — previously those were free retries.
    let Some(code) = joining.begin_attempt() else {
        // The window closed between the handshake and here.
        return;
    };
    let Some(peer) = peer_of(tls) else {
        return;
    };
    let binding = {
        let (_, conn) = tls.get_ref();
        match crate::pairing::binding_from_server(conn) {
            Ok(b) => b,
            Err(_) => return,
        }
    };

    // The failure arm is deliberately empty: `begin_attempt` already charged
    // this guess, so there is nothing left to record when the ceremony fails.
    if let Ok(paired) = crate::peer::pair::run(
        tls,
        crate::peer::pair::Role::Responder,
        identity,
        peer,
        &code,
        binding,
    )
    .await
    {
        // Single use: the invitation is spent whether or not the pin
        // write below succeeds, because the code has now been seen on the
        // wire by whoever answered.
        //
        // Losing this race means another ceremony already spent the
        // invitation. Both peers hold a valid code, so neither is
        // necessarily hostile — but "one invitation, one machine" is the
        // property, and admitting the loser would quietly break it.
        let Some(consumed) = joining.consume() else {
            eprintln!(
                "refusing {}: that invitation was already used by another machine. \
                     Mint a fresh one to add this node.",
                paired.node.short()
            );
            return;
        };
        if let Err(e) = crate::fleet::record_pairing(
            pins,
            paired.node,
            &paired.public_key,
            atlasctl_protocol::fleet::DisplayName::new(&paired.name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            None,
            // The grant the human chose when they minted this invitation,
            // written with the pin so consent and trust land atomically.
            consumed.allow_control,
        ) {
            // Announcing a pairing whose pin never reached disk is how a
            // fleet ends up with one side believing it is paired and the
            // other rejecting it on the next connection, with nothing
            // anywhere saying why.
            eprintln!(
                "pairing with {} completed but could not be recorded: {e:#}. \
                     The peer believes it is paired; this machine does not.",
                paired.node.short()
            );
            return;
        }
        let _ = fleet;
        // The words, not just the name. The machine that DIALLED shows these to
        // its operator and asks them to compare — and until now this side
        // printed nothing to compare against, so the comparison was one-sided
        // and the question the other dialog asked could not be answered.
        //
        // This is a log line rather than a prompt because this side is
        // typically headless and unattended: the ceremony is authorised by the
        // invitation, which a human minted here minutes ago. The words let
        // someone who wants to check, check.
        eprintln!(
            "paired with {} ({}) — verification words: {}",
            // Sanitised: the name is the joining peer's own claim, and this
            // line goes into the journal, where a control sequence is just as
            // effective at rewriting what a reader sees.
            atlasctl_protocol::fleet::DisplayName::new(&paired.name).as_str(),
            paired.node.short(),
            paired.verification
        );
    }
}

/// Ask each paired peer how it is.
fn spawn_peer_poll(
    fleet: Arc<crate::fleet::LocalFleet>,
    identity: Arc<crate::identity::Identity>,
    pins: crate::identity::PinStore,
    events: broadcast::Sender<ServerMsg>,
    port: u16,
    accelerator: String,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PEER_POLL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Ok(peers) = fleet.dialable_peers() else {
                continue;
            };
            for (id, dial) in peers {
                let addr = dial.addr;
                // Both rules — structural IPv6 handling and "the peer's port
                // first" — live in `peer::reach::dial_socket`, where they are
                // tested. They were inline here, inside a spawned loop with no
                // unit seam, so a mutation that dropped the advertised port
                // broke nothing.
                let Some(sock) = crate::peer::reach::dial_socket(&addr, dial.port, port) else {
                    continue;
                };
                let link = fleet.classify_peer_address(&addr);
                match crate::peer::link::query(
                    &identity,
                    pins.clone(),
                    sock,
                    id,
                    link,
                    &crate::peer::link::SelfIntro::new(fleet.can_launch(), &accelerator),
                    &fleet.local_addresses(),
                )
                .await
                {
                    Ok(report) => {
                        // The digest is recorded beside the report, from the
                        // same authenticated exchange. `None` = the peer did
                        // not say (old build) and its previous claims stand;
                        // `Some` = its complete current statement, replacing
                        // them wholesale. Vouches deliberately survive the
                        // error arm below: `clear_report` keeps them, and
                        // routing stays safe because choose_voucher requires
                        // a live report.
                        if let Some(digest) = report.vouched.clone() {
                            fleet.record_vouches(id, digest);
                        }
                        // Where the peer actually IS, persisted from the one
                        // place that has proven it: this exchange completed
                        // mutual TLS against the pinned key, so `sock` is an
                        // address that machine really answers on. `observe`
                        // used to do this from an unauthenticated beacon, which
                        // let anything on the LAN rewrite a trusted peer's
                        // address by announcing its (public) fingerprint.
                        let _ = crate::fleet::remember_address(&pins, id, &sock.ip().to_string());
                        fleet.record_report(report);
                        if let Some(node) = fleet.nodes().into_iter().find(|n| n.id == id) {
                            let _ = events.send(ServerMsg::FleetEvent {
                                event: FleetEvent::NodeChanged {
                                    node: Box::new(node),
                                },
                            });
                        }
                    }
                    // A peer that is switched off is the normal state of a
                    // fleet, not an error. Forget what it last said so the
                    // interface stops presenting stale vitals as current.
                    Err(_) => fleet.clear_report(id),
                }
            }
        }
    });
}

/// Something that can both advertise and browse.
///
/// One trait so the caller passes a single object; the two halves are separate
/// traits because a hardened deployment may want to browse without advertising.
pub trait DiscoveryPair: Advertiser + DiscoveryBrowser {}

impl<T: Advertiser + DiscoveryBrowser> DiscoveryPair for T {}

/// Advertise this node, and record what we hear.
fn spawn_discovery(
    fleet: Arc<LocalFleet>,
    events: broadcast::Sender<ServerMsg>,
    discovery: Arc<dyn DiscoveryPair>,
    beacon: Beacon,
) {
    tokio::task::spawn_blocking(move || {
        // Browse before advertising, so our own record does not race the
        // subscription and get missed.
        let rx = match discovery.browse() {
            Ok(rx) => rx,
            Err(e) => {
                // Multicast is filtered on plenty of networks. Say so once and
                // stop, rather than retrying forever against a switch that is
                // never going to answer.
                eprintln!(
                    "discovery unavailable: {e}\n  peers will not appear on their own; \
                     use `atlasctl peer add <host>` instead"
                );
                return;
            }
        };
        if let Err(e) = discovery.advertise(&beacon) {
            eprintln!("could not advertise on this network: {e}");
        }

        while let Ok(event) = rx.recv() {
            match event {
                DiscoveryEvent::Found(b) => {
                    let id = b.id;
                    let known = fleet.nodes().iter().any(|n| n.id == id);
                    fleet.observe(*b);
                    // Only announce genuinely new machines. A beacon refreshes
                    // every few seconds, and re-announcing an unchanged node
                    // would make the interface flicker for no reason.
                    if !known && let Some(node) = fleet.nodes().into_iter().find(|n| n.id == id) {
                        let _ = events.send(ServerMsg::FleetEvent {
                            event: FleetEvent::NodeChanged {
                                node: Box::new(node),
                            },
                        });
                    }
                }
                DiscoveryEvent::Lost(id) => {
                    let _ = events.send(ServerMsg::FleetEvent {
                        event: FleetEvent::NodeGone { node: id },
                    });
                }
            }
        }
    });
}
