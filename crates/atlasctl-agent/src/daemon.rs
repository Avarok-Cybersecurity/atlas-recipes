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
    spawn_prune(fleet, events);
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

/// Sample this machine and push the result.
fn spawn_vitals(fleet: Arc<LocalFleet>, events: broadcast::Sender<ServerMsg>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(VITALS_INTERVAL);
        // If a sample takes longer than the interval, skip rather than queue:
        // catching up on stale samples is worse than missing them.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            // Nobody is watching, so do not spawn a process to find out.
            if events.receiver_count() == 0 {
                continue;
            }
            let fleet = Arc::clone(&fleet);
            // Sampling shells out, so it must not run on the async runtime.
            let sampled = tokio::task::spawn_blocking(move || fleet.local_vitals_and_id()).await;
            if let Ok(Some((id, vitals))) = sampled {
                let _ = events.send(ServerMsg::FleetEvent {
                    event: FleetEvent::Vitals {
                        node: id,
                        vitals: Box::new(vitals),
                    },
                });
            }
        }
    });
}

/// Age out sightings of machines that have gone away.
fn spawn_prune(fleet: Arc<LocalFleet>, events: broadcast::Sender<ServerMsg>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(PRUNE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let before: Vec<_> = fleet.nodes().into_iter().map(|n| n.id).collect();
            fleet.prune();
            let after: Vec<_> = fleet.nodes().into_iter().map(|n| n.id).collect();
            for gone in before.iter().filter(|id| !after.contains(id)) {
                let _ = events.send(ServerMsg::FleetEvent {
                    event: FleetEvent::NodeGone { node: *gone },
                });
            }
        }
    });
}
