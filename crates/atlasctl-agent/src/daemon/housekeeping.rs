// SPDX-License-Identifier: AGPL-3.0-only

//! The two housekeeping timers: sample this machine, and forget nodes that
//! stopped talking.
//!
//! Split out of `daemon.rs` when it crossed the 500-line cap. The seam is real
//! rather than convenient: everything here is a timer over *local* state and
//! touches no socket, no TLS and no peer, while its parent is entirely about
//! talking to other machines. Neither half calls the other.

use super::*;

/// Sample this machine and push the result.
pub(super) fn spawn_vitals(fleet: Arc<LocalFleet>, events: broadcast::Sender<ServerMsg>) {
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
            let sampled = tokio::task::spawn_blocking(move || {
                // Both shell out, so they share the blocking hop.
                let changed = fleet.refresh_running().is_some();
                // Re-read after refreshing, so a node whose running model just
                // changed is described as it is now rather than as it was.
                let local = changed.then(|| fleet.nodes().into_iter().next());
                (fleet.local_vitals_and_id(), local)
            })
            .await;
            let sampled = match sampled {
                Ok((v, local)) => {
                    if let Some(Some(node)) = local {
                        let _ = events.send(ServerMsg::FleetEvent {
                            event: FleetEvent::NodeChanged {
                                node: Box::new(node),
                            },
                        });
                    }
                    Ok(v)
                }
                Err(e) => Err(e),
            };
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
pub(super) fn spawn_prune(fleet: Arc<LocalFleet>, events: broadcast::Sender<ServerMsg>) {
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
