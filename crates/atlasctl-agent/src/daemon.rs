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
pub fn spawn_peer_work(
    fleet: Arc<crate::fleet::LocalFleet>,
    identity: Arc<crate::identity::Identity>,
    pins: crate::identity::PinStore,
    events: broadcast::Sender<ServerMsg>,
    peer_port: u16,
    rank: Arc<dyn RankService>,
    joining: Arc<crate::joining::JoinWindow>,
) {
    spawn_peer_listener(
        Arc::clone(&fleet),
        Arc::clone(&identity),
        pins.clone(),
        peer_port,
        rank,
        joining,
    );
    spawn_peer_poll(fleet, identity, pins, events, peer_port);
}

/// Accept connections from paired peers.
fn spawn_peer_listener(
    fleet: Arc<crate::fleet::LocalFleet>,
    identity: Arc<crate::identity::Identity>,
    pins: crate::identity::PinStore,
    port: u16,
    rank: Arc<dyn RankService>,
    joining: Arc<crate::joining::JoinWindow>,
) {
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
                    "peer channel disabled: could not bind {port}: {e}\n                       other machines will not be able to reach this one"
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
            let rank = Arc::clone(&rank);
            let joining = Arc::clone(&joining);
            let pins = pins.clone();
            let identity = Arc::clone(&identity);
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

                let vitals = fleet.local_vitals_and_id().map(|(_, v)| v);
                if crate::peer::link::serve_query(
                    &mut tls,
                    crate::discovery::local_display_name().as_str(),
                    fleet.can_launch(),
                    "",
                    &crate::discovery::local_os(),
                    vitals,
                    &fleet.local_addresses(),
                )
                .await
                .is_err()
                {
                    return;
                }
                // The peer may go on to ask this rank to describe what it would
                // run. Rendering happens here, on the machine that would
                // execute it, from this machine's own vendored recipe.
                serve_rank_requests(&mut tls, &rank).await;
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
        if !joining.consume() {
            eprintln!(
                "refusing {}: that invitation was already used by another machine. \
                     Mint a fresh one to add this node.",
                paired.node.short()
            );
            return;
        }
        if let Err(e) = crate::fleet::record_pairing(
            pins,
            paired.node,
            &paired.public_key,
            atlasctl_protocol::fleet::DisplayName::new(&paired.name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            None,
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
        eprintln!("paired with {} ({})", paired.name, paired.node.short());
    }
}

/// Ask each paired peer how it is.
fn spawn_peer_poll(
    fleet: Arc<crate::fleet::LocalFleet>,
    identity: Arc<crate::identity::Identity>,
    pins: crate::identity::PinStore,
    events: broadcast::Sender<ServerMsg>,
    port: u16,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PEER_POLL_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let Ok(peers) = fleet.dialable_peers() else {
                continue;
            };
            for (id, addr) in peers {
                let Ok(sock) = format!("{addr}:{port}").parse() else {
                    continue;
                };
                let link = fleet.classify_peer_address(&addr);
                match crate::peer::link::query(
                    &identity,
                    pins.clone(),
                    sock,
                    id,
                    link,
                    &crate::peer::link::SelfIntro::new(fleet.can_launch(), ""),
                    &fleet.local_addresses(),
                )
                .await
                {
                    Ok(report) => {
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

/// Answer a paired peer's questions about what this rank would run.
///
/// Only reached after the TLS verifier confirmed the caller is pinned, so there
/// is no authorization decision here — only rendering, from this machine's own
/// copy of the recipe.
async fn serve_rank_requests<S>(stream: &mut S, rank: &Arc<dyn RankService>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use crate::peer::wire::{PeerFrame, read_frame, write_frame};
    loop {
        let Ok(frame) = read_frame(stream).await else {
            return;
        };
        let reply = match frame {
            PeerFrame::PreviewRank { assignment } => match rank.render(&assignment) {
                Ok((command, unmapped)) => PeerFrame::RankPreviewed { command, unmapped },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            PeerFrame::Prepare { assignment, epoch } => PeerFrame::Prepared {
                reply: rank.prepare(&epoch, &assignment),
                epoch,
            },
            // Commit deliberately carries no assignment: what starts is what
            // this machine rendered and stored at prepare time, so a head
            // compromised between the phases cannot substitute anything.
            PeerFrame::Commit { epoch } => match rank.commit(&epoch) {
                Ok(container) => PeerFrame::Committed { epoch, container },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            // Abort is acknowledged rather than answered with a result: the
            // head is already rolling back, and a failure to release must not
            // mask whatever caused the rollback.
            // Acknowledged whether or not the container was there: a rollback
            // asking twice, or asking about a rank that never started, is an
            // ordinary race and not something the head can act on.
            PeerFrame::IsRankAlive { container } => PeerFrame::RankLiveness {
                // Unaskable is not alive: a rank whose state we cannot read
                // must not be counted as part of a whole cluster.
                running: rank.alive(&container).unwrap_or(false),
                container,
            },
            PeerFrame::StopRank { container } => {
                let _ = rank.stop(&container);
                PeerFrame::RankStopped { container }
            }
            PeerFrame::Abort { epoch } => {
                rank.abort(&epoch);
                PeerFrame::Aborted { epoch }
            }
            _ => return,
        };
        if write_frame(stream, &reply).await.is_err() {
            return;
        }
    }
}
