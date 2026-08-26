// SPDX-License-Identifier: AGPL-3.0-only

//! Turning what this agent knows into the list a browser renders.
//!
//! Split from [`super`] for size. The precedence it encodes is the whole point
//! and is worth stating plainly:
//!
//! 1. A **peer report** — something holding the key we pinned, said over the
//!    authenticated channel — wins outright. Its vitals and its link class are
//!    evidence.
//! 2. A **beacon sighting** places a machine and nothing more. Its link class is
//!    `Unverified`, never a guess, and it carries no vitals at all.
//! 3. A **pin with a remembered address** keeps a paired machine listed while it
//!    is switched off, because it is still part of your fleet when it is off.
//!
//! Anything seen but not pinned is listed, trusted by nobody, and shows no
//! vitals: telemetry from a machine you have not paired is not evidence about
//! your fleet.

use super::{LocalFleet, PairOutcome, UNREACHABLE_AFTER};
use crate::discovery::Beacon;
use crate::fleet::FleetView;
use anyhow::Result;
use atlasctl_protocol::fleet::{Launchability, NodeAddress, NodeDescriptor, NodeId, PairingState};

impl FleetView for LocalFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        let mut out = vec![self.local_node()];
        let pinned = self.pins.load().unwrap_or_default();
        let seen = self.lock_seen();
        let alerts = self.alerts.lock().ok();

        // Start from the pin store, so a paired machine that is switched off is
        // still listed. A fleet that forgets its members when they sleep is not
        // a fleet.
        let reports = self.reports.lock().ok();
        for (id, pin) in &pinned {
            let sighting = seen.as_ref().and_then(|s| s.get(id));
            let report = reports.as_ref().and_then(|r| r.get(id));
            // A completed handshake is better evidence of liveness than a
            // beacon in either direction: a wedged agent can still broadcast,
            // and a healthy one goes quiet the moment a switch filters
            // multicast.
            let fresh =
                report.is_some() || sighting.is_some_and(|s| s.at.elapsed() < UNREACHABLE_AFTER);
            out.push(NodeDescriptor {
                id: *id,
                name: sighting.map_or_else(|| pin.name.clone(), |s| s.beacon.name.clone()),
                is_local: false,
                pairing: if fresh {
                    PairingState::Paired
                } else {
                    PairingState::Unreachable
                },
                // A live sighting wins; otherwise fall back to where this peer
                // was last known to be, so a restart does not make a paired
                // machine look unreachable-and-addressless.
                addresses: report
                    .map(|r| {
                        // Reached and authenticated, so the link class is ours
                        // to state rather than a guess.
                        vec![NodeAddress {
                            iface: String::new(),
                            addr: pin.last_address.clone().unwrap_or_default(),
                            class: r.link,
                            speed_mbps: None,
                            rdma: matches!(
                                r.link,
                                atlasctl_protocol::fleet::LinkClass::Roce
                                    | atlasctl_protocol::fleet::LinkClass::InfiniBand
                            ),
                        }]
                    })
                    .unwrap_or_else(|| {
                        sighting.map_or_else(
                            || {
                                pin.last_address
                                    .as_ref()
                                    .map(|a| {
                                        vec![NodeAddress {
                                            iface: String::new(),
                                            addr: a.clone(),
                                            class: atlasctl_protocol::fleet::LinkClass::Unverified,
                                            speed_mbps: None,
                                            rdma: false,
                                        }]
                                    })
                                    .unwrap_or_default()
                            },
                            |s| addresses_of(&s.beacon),
                        )
                    }),
                launchability: sighting.map_or_else(
                    || Launchability::no("not reachable right now"),
                    |s| {
                        if s.beacon.can_launch {
                            Launchability::yes()
                        } else {
                            Launchability::no("this node reports it cannot run models")
                        }
                    },
                ),
                agent_version: String::new(),
                accelerator: report.map_or_else(
                    || sighting.map_or_else(String::new, |s| s.beacon.accelerator.clone()),
                    |r| r.accelerator.clone(),
                ),
                // Only ever from the authenticated channel. A peer we have not
                // spoken to reports none, rather than a beacon's word for it.
                vitals: report.and_then(|r| r.vitals.clone()),
                alerts: alerts
                    .as_ref()
                    .and_then(|a| a.get(id).cloned())
                    .unwrap_or_default(),
                running: None,
            });
        }

        // Then anything seen but not pinned: visible, and trusted by nobody.
        if let Some(seen) = seen.as_ref() {
            for (id, s) in seen.iter() {
                if pinned.contains_key(id) {
                    continue;
                }
                out.push(NodeDescriptor {
                    id: *id,
                    name: s.beacon.name.clone(),
                    is_local: false,
                    pairing: PairingState::Discovered,
                    addresses: addresses_of(&s.beacon),
                    launchability: if s.beacon.can_launch {
                        Launchability::yes()
                    } else {
                        Launchability::no("this node reports it cannot run models")
                    },
                    agent_version: String::new(),
                    accelerator: s.beacon.accelerator.clone(),
                    vitals: None,
                    alerts: Vec::new(),
                    running: None,
                });
            }
        }
        out
    }

    fn pair(&self, node: NodeId, code: &str) -> Result<PairOutcome> {
        anyhow::ensure!(
            crate::pairing::looks_like_code(code),
            "a pairing code is {} digits",
            crate::pairing::CODE_DIGITS
        );
        let seen = self
            .lock_seen()
            .and_then(|s| s.get(&node).cloned())
            .ok_or_else(|| anyhow::anyhow!("that node is not visible on this network"))?;

        // The ceremony itself runs over the peer channel; this is the point
        // where that would be driven. Until the peer transport is wired in,
        // refuse rather than pretend: a pairing that silently succeeded without
        // a key exchange would write a pin that means nothing.
        anyhow::bail!(
            "cannot reach {} to pair: the peer channel is not connected",
            seen.beacon.name
        )
    }

    fn unpair(&self, node: NodeId) -> Result<bool> {
        self.pins.remove(node)
    }
}

/// Addresses a beacon advertised, as protocol addresses.
///
/// A beacon carries no link classification — it is unauthenticated, and letting
/// a stranger tell you their link is RoCE would let them talk their way to the
/// top of your preference order.
///
/// The class is therefore `Unverified` rather than a guess. Guessing "ethernet"
/// was worse than saying nothing: it told the operator of a 200 Gb RoCE fabric
/// that their cluster would run on ethernet. It is resolved once the peer is
/// reached over the authenticated channel and describes itself.
fn addresses_of(beacon: &Beacon) -> Vec<NodeAddress> {
    beacon
        .addresses
        .iter()
        .map(|a| NodeAddress {
            iface: String::new(),
            addr: a.to_string(),
            class: atlasctl_protocol::fleet::LinkClass::Unverified,
            speed_mbps: None,
            rdma: false,
        })
        .collect()
}
