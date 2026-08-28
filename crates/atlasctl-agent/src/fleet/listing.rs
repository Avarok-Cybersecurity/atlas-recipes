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
//! 4. A **vouch** — a pinned peer's claim about a peer of ITS own — contributes
//!    a descriptor only when none of the first-hand tiers above know the node
//!    at all. It lists as [`PairingState::Vouched`] with `vouched_by` naming
//!    the claimant, because rendering a claim with the same face as evidence
//!    would lie to the operator about who verified what. When first-hand
//!    evidence exists, the vouch contributes exactly one thing: `vouched_by`
//!    as labeled corroboration. Fields are never blended across tiers.
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
        // Snapshotted before any guard below is taken: building it re-locks
        // the report cache, and a std mutex re-acquired on this same thread
        // would deadlock, not error.
        let vouches = self.vouch_view();
        let seen = self.lock_seen();
        let alerts = self.alerts.lock().ok();

        // Start from the pin store, so a paired machine that is switched off is
        // still listed. A fleet that forgets its members when they sleep is not
        // a fleet.
        let reports = self.reports.lock().ok();
        for (id, pin) in &pinned {
            let sighting = seen.as_ref().and_then(|s| s.get(id));
            let report = reports.as_ref().and_then(|r| r.get(id)).map(|(r, _)| r);
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
                // First-hand knowledge wins every attribute above, but a
                // vouch for a node we ALSO pinned is still worth labeling:
                // corroboration, and the operator's map of who claims what.
                vouched_by: vouches.get(id).map(|(voucher, _, _)| *voucher),
                // A pinned target is dialled directly (the router's rule O2);
                // claiming a relay here would show a route never taken.
                reached_via: None,
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
                    // A sighting is first-hand placement; a vouch for the
                    // same node is corroboration worth labeling, nothing
                    // more — no field above came from it.
                    vouched_by: vouches.get(id).map(|(voucher, _, _)| *voucher),
                    // Seen on the wire ourselves: nothing routes through
                    // anyone.
                    reached_via: None,
                });
            }
        }

        // Rung 4: nodes known ONLY through a voucher's claim. Everything on
        // this descriptor is that claim, labeled as such — `Vouched`, never a
        // treatment shared with `Paired`, because the one thing this tier
        // must not do is make second-hand knowledge look verified.
        for (target, (voucher, claim, route)) in &vouches {
            if pinned.contains_key(target)
                || seen.as_ref().is_some_and(|s| s.contains_key(target))
                || *target == self.id()
            {
                continue;
            }
            out.push(NodeDescriptor {
                id: *target,
                name: claim.name.clone(),
                is_local: false,
                pairing: PairingState::Vouched,
                // Display data only: control toward a vouched node rides its
                // voucher, never a dial to an address someone else relayed.
                addresses: claim.addresses.clone(),
                launchability: as_launchability(claim.can_launch),
                agent_version: String::new(),
                accelerator: claim.accelerator.clone(),
                os: claim.os.clone(),
                // Second-hand vitals are shown only with a stated age inside
                // the same staleness bound everything else uses. Vitals whose
                // age is missing are dropped outright — an unknown age is not
                // zero, and rendering them would present old data as fresh.
                vitals: match claim.vitals_age_s {
                    Some(age) if age <= UNREACHABLE_AFTER.as_secs() => claim.vitals.clone(),
                    _ => None,
                },
                alerts: Vec::new(),
                running: None,
                vouched_by: Some(*voucher),
                reached_via: *route,
            });
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
        //
        // EVERY address, in the order advertised, not just the first. The
        // advertiser ranks its own links best-first, so the head of that list
        // is its fabric — and its fabric is frequently the one link the dialler
        // cannot reach. A laptop pairing with a DGX is handed 10.10.10.1 (RoCE,
        // rank 4) ahead of the LAN address it actually shares, so dialling only
        // the first timed out against a machine sitting on the same switch.
        // Preference is still honoured; it is now a preference rather than the
        // only attempt.
        let addrs: Vec<std::net::SocketAddr> = seen
            .beacon
            .addresses
            .iter()
            .map(|ip| std::net::SocketAddr::new(*ip, seen.beacon.peer_port))
            .collect();
        anyhow::ensure!(
            !addrs.is_empty(),
            "{} advertised no address to dial",
            seen.beacon.name
        );

        let (addr, paired) = dial_first_reachable(driver.as_ref(), &addrs, code)?;

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

    fn pair_at(&self, target: &str, code: &str) -> Result<PairOutcome> {
        anyhow::ensure!(
            crate::pairing::looks_like_code(code),
            "a pairing code is {} digits",
            crate::pairing::CODE_DIGITS
        );

        let Some(driver) = self.pairing.as_ref() else {
            anyhow::bail!("this agent has no peer transport, so it cannot run a pairing ceremony");
        };

        // A name can resolve to several addresses, and a machine on two subnets
        // usually does. Try them all rather than whichever the resolver put
        // first.
        let addrs = crate::discovery::resolve_manual(target, crate::peer::DEFAULT_PEER_PORT)?;
        anyhow::ensure!(!addrs.is_empty(), "{target} resolved to no address");

        let (addr, paired) = dial_first_reachable(driver.as_ref(), &addrs, code)?;

        // No identity assertion here, deliberately. `pair` checks that the
        // machine which answered is the one the operator SELECTED from a list;
        // here they selected an address, and there is no prior claim about who
        // lives at it to check against. The identity that answered goes back in
        // the outcome, and the operator judges it at the word comparison — the
        // one step where a human is already deciding whether to trust this
        // machine. Inventing an expectation to assert would only assert itself.
        Ok(PairOutcome {
            node: paired.node,
            public_key: paired.public_key,
            name: paired.name,
            address: addr.ip().to_string(),
            verification: paired.verification,
        })
    }

    fn trust(&self, outcome: &PairOutcome, allow_control: bool) -> Result<()> {
        super::record_pairing(
            &self.pins,
            outcome.node,
            &outcome.public_key,
            atlasctl_protocol::fleet::DisplayName::new(&outcome.name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            Some(outcome.address.clone()),
            allow_control,
        )
    }

    fn unpair(&self, node: NodeId) -> Result<bool> {
        // Its vouches go with the pin, atomically from the caller's view:
        // trust withdrawn from a machine withdraws every claim it made, so no
        // route or listing row survives on the word of a peer no longer
        // trusted to say anything.
        self.clear_vouches(node);
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

/// Dial each candidate in order until one completes the ceremony.
///
/// Returns the address that answered together with its outcome, so the pin
/// records where the machine actually was rather than where it was first
/// guessed to be.
///
/// The errors are accumulated and reported together. Reporting only the last
/// one would name whichever address happened to sort last — usually the least
/// interesting failure — and hide the fact that several links were tried.
fn dial_first_reachable(
    driver: &dyn super::PeerPairing,
    addrs: &[std::net::SocketAddr],
    code: &str,
) -> Result<(std::net::SocketAddr, crate::peer::pair::Paired)> {
    // The walk and its stop rule live in `peer::reach`, shared with the
    // joining direction: only a failure to REACH the peer earns another
    // address, because every address here is the same machine and a refusal
    // has already spent one of the code's three attempts.
    // `{e:#}`, not `.context(…)`: the accumulated reasons belong in the
    // message itself. Behind a context they only appear when something prints
    // the whole chain, and the caller nearest the operator prints the top.
    crate::peer::reach::walk(addrs, |addr| driver.pair(addr, code))
        .map_err(|e| anyhow::anyhow!("could not pair over any advertised address — {e:#}"))
}
