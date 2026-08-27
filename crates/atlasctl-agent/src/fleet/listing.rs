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

/// Turn a peer's own answer about itself into a launchability.
///
/// One function so the two places that decide this cannot drift, and so the
/// wording an operator reads is the same whichever source supplied it.
fn as_launchability(can_launch: bool) -> Launchability {
    if can_launch {
        Launchability::yes()
    } else {
        Launchability::no("this node reports it cannot run models")
    }
}

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
                        // What the peer said about its own links, when it said
                        // anything: it is the only source that knows subnets,
                        // and it came over the authenticated channel.
                        if !r.addresses.is_empty() {
                            return r.addresses.clone();
                        }
                        // Reached and authenticated, so the link class is ours
                        // to state rather than a guess.
                        vec![NodeAddress {
                            iface: String::new(),
                            addr: pin.last_address.clone().unwrap_or_default(),
                            class: r.link,
                            speed_mbps: None,
                            // A pin records a host address, never its subnet.
                            prefix_len: 0,
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
                                            // Last known host address; no subnet.
                                            prefix_len: 0,
                                            rdma: false,
                                        }]
                                    })
                                    .unwrap_or_default()
                            },
                            |s| addresses_of(&s.beacon),
                        )
                    }),
                // Same precedence as everything else on this descriptor, and
                // it was missing here: launchability came from the beacon
                // alone. A paired machine on a network that filters multicast
                // has no beacon — the case `peer add` exists for — so it
                // reported "not reachable right now" and could not be given a
                // rank, moments after completing an authenticated handshake.
                launchability: report.map_or_else(
                    || {
                        sighting.map_or_else(
                            || Launchability::no("not reachable right now"),
                            |s| as_launchability(s.beacon.can_launch),
                        )
                    },
                    |r| as_launchability(r.can_launch),
                ),
                agent_version: String::new(),
                accelerator: report.map_or_else(
                    || sighting.map_or_else(String::new, |s| s.beacon.accelerator.clone()),
                    |r| r.accelerator.clone(),
                ),
                // Only from the authenticated channel. A beacon does not carry
                // it, and inventing one would put a guess where the interface
                // shows a fact.
                os: report.map_or_else(String::new, |r| r.os.clone()),
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
                    launchability: as_launchability(s.beacon.can_launch),
                    agent_version: String::new(),
                    accelerator: s.beacon.accelerator.clone(),
                    // A machine we have never spoken to has told us nothing we
                    // can believe about itself.
                    os: String::new(),
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

        let Some(driver) = self.pairing.as_ref() else {
            // Refuse rather than pretend. A pairing that reported success
            // without a key exchange would write a pin that means nothing,
            // which is worse than not pairing at all.
            anyhow::bail!("this agent has no peer transport, so it cannot run a pairing ceremony");
        };

        // The beacon says where it is; the ceremony decides whether it is who
        // it claims. An address from an unauthenticated beacon is safe to dial
        // precisely because dialling it proves nothing on its own.
        let addr = seen
            .beacon
            .addresses
            .first()
            .map(|ip| std::net::SocketAddr::new(*ip, seen.beacon.peer_port))
            .ok_or_else(|| anyhow::anyhow!("{} advertised no address to dial", seen.beacon.name))?;

        let paired = driver.pair(addr, code)?;

        // The ceremony authenticates the peer; this checks it is the peer the
        // operator asked for. Without it, a machine answering on that address
        // could be pinned under the identity of the one that was chosen.
        anyhow::ensure!(
            paired.node == node,
            "reached {} at {addr}, but {} was selected",
            paired.node.short(),
            node.short()
        );

        // No pin is written here. The exchange proves both machines derived the
        // same key; it does not prove the operator meant to trust this one, and
        // that is what the words are for. `trust` writes it once they say so.
        Ok(PairOutcome {
            node: paired.node,
            public_key: paired.public_key,
            name: paired.name,
            address: addr.ip().to_string(),
            verification: paired.verification,
        })
    }

    fn trust(&self, outcome: &PairOutcome) -> Result<()> {
        super::record_pairing(
            &self.pins,
            outcome.node,
            &outcome.public_key,
            atlasctl_protocol::fleet::DisplayName::new(&outcome.name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            Some(outcome.address.clone()),
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
            // A beacon carries observed host addresses, not interface subnets.
            prefix_len: 0,
            rdma: false,
        })
        .collect()
}
