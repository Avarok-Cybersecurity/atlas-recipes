// SPDX-License-Identifier: AGPL-3.0-only

//! Second-hand fleet knowledge: what pinned peers say about THEIR pins.
//!
//! Split from [`super`] for size — `fleet.rs` holds the first-hand tables and
//! this file holds the second-hand one — and because the seam is a trust
//! boundary worth a file of its own: everything in here is a CLAIM by a
//! voucher, never evidence this agent gathered itself. The table is runtime
//! state like reports and sightings, never persisted: a restart re-learns it
//! within one poll interval, and replaying stale claims from disk after the
//! voucher retracted them would be worse than a blank map.
//!
//! The one-hop rule lives here structurally: this table is written from
//! received digests and read by the listing and the router, and it is NOT a
//! source [`crate::peer::link::fleet_digest`] reads — so a vouch can never be
//! re-vouched, which is what stops gossip flooding without a TTL anyone could
//! rewrite.

use super::{LocalFleet, UNREACHABLE_AFTER};
use atlasctl_protocol::fleet::{MAX_VOUCHED, NodeId, VouchedPeer};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Instant;

/// One voucher's claim about a target, and when this agent recorded it.
type RecordedClaim = (VouchedPeer, Instant);

/// Everything currently claimed about one target, by voucher.
type ClaimsByVoucher = BTreeMap<NodeId, RecordedClaim>;

/// Second-hand fleet knowledge: target -> (voucher -> claim + when recorded).
#[derive(Debug, Default)]
pub struct VouchTable {
    claims: Mutex<BTreeMap<NodeId, ClaimsByVoucher>>,
}

impl LocalFleet {
    /// Record a voucher's digest, REPLACING its previous claims wholesale.
    ///
    /// Replacement, never a merge: a digest is the voucher's complete current
    /// statement, so a node it stopped vouching for — "dgx1 unpaired dgx2" —
    /// disappears within one poll instead of lingering as a ghost route.
    ///
    /// A digest longer than [`MAX_VOUCHED`] is refused wholesale, keeping the
    /// voucher's previous well-formed statement: truncating would silently
    /// hide fleet members, and guessing WHICH entries a misbehaving speaker
    /// meant is not this agent's job. Entries naming this agent or the
    /// speaker itself are dropped — the mirror of the `observe()` self-beacon
    /// guard — so a digest cannot make a machine its own voucher or list the
    /// receiver as a stranger in its own fleet.
    pub fn record_vouches(&self, via: NodeId, digest: Vec<VouchedPeer>) {
        if digest.len() > MAX_VOUCHED {
            return;
        }
        let Ok(mut claims) = self.vouches.claims.lock() else {
            return;
        };
        for by_voucher in claims.values_mut() {
            by_voucher.remove(&via);
        }
        let now = Instant::now();
        for entry in digest {
            if entry.node == self.id() || entry.node == via {
                continue;
            }
            claims
                .entry(entry.node)
                .or_default()
                .insert(via, (entry, now));
        }
        claims.retain(|_, by_voucher| !by_voucher.is_empty());
    }

    /// Drop every claim a voucher has made.
    ///
    /// Called when its report is cleared and when it is unpaired: a voucher
    /// this agent can no longer hear — or no longer trusts — must not keep
    /// steering routes through claims nobody can re-confirm.
    pub fn clear_vouches(&self, via: NodeId) {
        let Ok(mut claims) = self.vouches.claims.lock() else {
            return;
        };
        for by_voucher in claims.values_mut() {
            by_voucher.remove(&via);
        }
        claims.retain(|_, by_voucher| !by_voucher.is_empty());
    }

    /// The relay a control verb aimed at `target` would ride, when one exists.
    ///
    /// THE routing rule, stated once (SSOT): the listing's `reached_via` and
    /// the session router must call this same function, so the UI never
    /// displays a route the router would not take. Eligible vouchers are
    /// those this agent can currently hear (fresh report) that claim the
    /// target reachable; among them, the one whose leg FROM US has the
    /// highest link rank wins, and ties break to the numerically smallest
    /// voucher id — an explicit rule, not map iteration order, because an
    /// unstable choice makes the UI reorder itself and a failure
    /// unreproducible.
    #[must_use]
    pub fn choose_voucher(&self, target: NodeId) -> Option<NodeId> {
        let claims = self.vouches.claims.lock().ok()?;
        let by_voucher = claims.get(&target)?;
        let reports = self.reports.lock().ok()?;

        let mut best: Option<(u8, NodeId)> = None;
        // BTreeMap iterates vouchers in byte order, so keeping the FIRST of
        // any rank (strict `>`) is exactly "ties break to the smallest id".
        for (voucher, (claim, _)) in by_voucher.iter() {
            if !claim.reachable {
                continue;
            }
            let Some((our_report, _)) = reports.get(voucher) else {
                continue;
            };
            let rank = our_report.link.rank();
            if best.is_none_or(|(r, _)| rank > r) {
                best = Some((rank, *voucher));
            }
        }
        best.map(|(_, voucher)| voucher)
    }

    /// The voucher whose claim describes `target`, with that claim.
    ///
    /// The voucher is [`Self::choose_voucher`]'s pick when a route exists, so
    /// the attributes shown are the ones behind the route that would be
    /// taken; with no eligible route it falls back to the smallest claiming
    /// voucher id — still deterministic, so the row does not flicker between
    /// vouchers across refreshes.
    pub(crate) fn vouch_of(&self, target: NodeId) -> Option<(NodeId, VouchedPeer)> {
        let routed = self.choose_voucher(target);
        let claims = self.vouches.claims.lock().ok()?;
        let by_voucher = claims.get(&target)?;
        let voucher = routed.or_else(|| by_voucher.keys().next().copied())?;
        let (claim, _) = by_voucher.get(&voucher)?;
        Some((voucher, claim.clone()))
    }

    /// Every node someone currently vouches for, in byte order.
    pub(crate) fn vouched_targets(&self) -> Vec<NodeId> {
        self.vouches
            .claims
            .lock()
            .map(|claims| claims.keys().copied().collect())
            .unwrap_or_default()
    }

    /// One pass over the whole table: target -> (voucher, claim, route).
    ///
    /// Built for the listing, which must not call back into this module while
    /// it holds the report-cache guard — [`Self::choose_voucher`] takes that
    /// same lock, and a std mutex re-acquired on the same thread deadlocks
    /// rather than erroring. The listing therefore takes this snapshot BEFORE
    /// locking anything of its own.
    pub(crate) fn vouch_view(&self) -> BTreeMap<NodeId, (NodeId, VouchedPeer, Option<NodeId>)> {
        let mut out = BTreeMap::new();
        for target in self.vouched_targets() {
            let route = self.choose_voucher(target);
            if let Some((voucher, claim)) = self.vouch_of(target) {
                out.insert(target, (voucher, claim, route));
            }
        }
        out
    }

    /// Drop claims old enough that nothing has re-confirmed them.
    ///
    /// Reached from the same housekeeping pass that prunes sightings, with
    /// the same threshold: a second staleness bound would give operators two
    /// notions of "gone" to reason about.
    pub(super) fn prune_vouches(&self) {
        let Ok(mut claims) = self.vouches.claims.lock() else {
            return;
        };
        for by_voucher in claims.values_mut() {
            by_voucher.retain(|_, (_, at)| at.elapsed() < UNREACHABLE_AFTER);
        }
        claims.retain(|_, by_voucher| !by_voucher.is_empty());
    }

    /// The live report cache with the moment each entry was recorded.
    ///
    /// For [`crate::peer::link::fleet_digest`], which must state how old the
    /// vitals it re-serializes are rather than presenting them as fresh.
    pub(crate) fn report_snapshot(
        &self,
    ) -> BTreeMap<NodeId, (crate::peer::link::PeerReport, Instant)> {
        self.reports.lock().map(|r| r.clone()).unwrap_or_default()
    }
}
