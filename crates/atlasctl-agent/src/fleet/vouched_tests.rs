// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for second-hand fleet knowledge: what a vouch may contribute, what
//! it may never do, and how the voucher for a route is chosen.

use super::tests::{Tmp, fleet_at};
use super::*;
use atlasctl_protocol::fleet::{LinkClass, Metric, VouchedPeer};

fn nid(n: u8) -> NodeId {
    NodeId::from_bytes([n; 32])
}

fn claim(node: NodeId, name: &str) -> VouchedPeer {
    VouchedPeer {
        node,
        name: DisplayName::new(name),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        addresses: Vec::new(),
        link: atlasctl_protocol::fleet::LinkClass::Roce,
        reachable: true,
        vitals: None,
        vitals_age_s: None,
    }
}

fn vitals() -> NodeVitals {
    NodeVitals {
        accelerator_util: Metric::reading(11.0),
        sm_clock_mhz: Metric::Unsupported,
        sm_clock_healthy_mhz: None,
        temperature_c: Metric::Unsupported,
        power_w: Metric::Unsupported,
        memory_used_frac: Metric::Unsupported,
        memory_total_bytes: Metric::Unsupported,
        disk_free_bytes: Metric::Unsupported,
        docker_ok: true,
        agent_uptime_s: 5,
    }
}

/// A report for a voucher, over the given link class, so `choose_voucher` has
/// a leg-from-us to rank.
fn report_over(
    id: NodeId,
    link: atlasctl_protocol::fleet::LinkClass,
) -> crate::peer::link::PeerReport {
    crate::peer::link::PeerReport {
        node: id,
        name: "voucher".to_owned(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link,
        addresses: Vec::new(),
        vouched: None,
        peer_version_max: crate::peer::wire::PEER_PROTOCOL_MAX,
    }
}

fn row(fleet: &LocalFleet, id: NodeId) -> Option<atlasctl_protocol::fleet::NodeDescriptor> {
    fleet.nodes().into_iter().find(|n| n.id == id)
}

#[test]
fn a_vouched_only_node_lists_second_hand_with_its_provenance() {
    let t = Tmp::new("vouchonly");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(voucher, vec![claim(nid(7), "spark-43fa")]);

    let node = row(&f, nid(7)).expect("a vouched node is listed");
    assert_eq!(
        node.pairing,
        PairingState::Vouched,
        "a claim must never wear the same state as verified evidence"
    );
    assert_eq!(node.vouched_by, Some(voucher));
    assert_eq!(
        node.reached_via,
        Some(voucher),
        "control toward a vouched node rides its voucher"
    );
    assert_eq!(node.name.as_str(), "spark-43fa");
}

#[test]
fn first_hand_evidence_beats_a_vouch_and_the_rows_merge_to_one() {
    // The precedence ladder, top rung against bottom: a vouched-only node
    // that is later paired directly must collapse to ONE row whose every
    // attribute is first-hand, with the vouch surviving only as the
    // `vouched_by` label. Two rows would let the stale claim shadow the
    // fresh evidence; blended fields would let a voucher edit a peer we can
    // ask ourselves.
    let t = Tmp::new("merge");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    let target = nid(7);
    f.record_report(report_over(voucher, LinkClass::Roce));
    // The voucher lies about the target's capability, to prove whose word
    // the merged row carries.
    let mut lied = claim(target, "wrong-name");
    lied.can_launch = false;
    f.record_vouches(voucher, vec![lied]);
    assert_eq!(
        row(&f, target).expect("vouched row").pairing,
        PairingState::Vouched
    );

    // Now pair with it directly and hear from it ourselves.
    record_pairing(
        &f.pins,
        target,
        "aa",
        DisplayName::new("spark-43fa"),
        0,
        Some("10.10.10.10".to_owned()),
    )
    .expect("pin");
    let mut first_hand = report_over(target, LinkClass::Roce);
    first_hand.name = "spark-43fa".to_owned();
    f.record_report(first_hand);

    let rows: Vec<_> = f.nodes().into_iter().filter(|n| n.id == target).collect();
    assert_eq!(rows.len(), 1, "one node, one row");
    let node = &rows[0];
    assert_eq!(node.pairing, PairingState::Paired);
    assert_eq!(node.name.as_str(), "spark-43fa", "first-hand name wins");
    assert!(
        node.launchability.can_launch,
        "the voucher's claim must not overwrite what the node itself said"
    );
    assert_eq!(
        node.reached_via, None,
        "a pinned node is dialled directly; the relay provenance drops"
    );
    assert_eq!(
        node.vouched_by,
        Some(voucher),
        "the vouch remains visible as labeled corroboration"
    );
}

#[test]
fn a_new_digest_replaces_the_speakers_claims_wholesale() {
    // One call, not an expiry: "dgx1 unpaired dgx2" must take effect the
    // moment dgx1's next digest arrives, or a retracted node lingers as a
    // ghost the UI still offers routes to.
    let t = Tmp::new("replace");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(
        voucher,
        vec![claim(nid(7), "seven"), claim(nid(8), "eight")],
    );
    assert!(row(&f, nid(7)).is_some());

    f.record_vouches(voucher, vec![claim(nid(8), "eight")]);
    assert!(
        row(&f, nid(7)).is_none(),
        "a retracted node must disappear in the same call"
    );
    assert!(row(&f, nid(8)).is_some());

    // And an affirmatively empty digest clears everything it ever said.
    f.record_vouches(voucher, Vec::new());
    assert!(row(&f, nid(8)).is_none());
}

#[test]
fn entries_naming_the_receiver_or_the_speaker_are_dropped() {
    // A digest cannot list this machine as a stranger in its own fleet, and
    // a speaker cannot be its own voucher — either would let one peer
    // manufacture provenance about identities the receiver already knows
    // first-hand by definition.
    let t = Tmp::new("selfspeaker");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(
        voucher,
        vec![
            claim(f.id(), "me-but-claimed"),
            claim(voucher, "self-vouch"),
        ],
    );

    assert_eq!(f.choose_voucher(f.id()), None);
    assert_eq!(f.choose_voucher(voucher), None);
    let local = row(&f, f.id()).expect("local row");
    assert_eq!(local.vouched_by, None, "nobody vouches for this machine");
    assert!(
        row(&f, voucher).is_none(),
        "the speaker gained no listing from vouching for itself"
    );
}

#[test]
fn an_oversized_digest_is_refused_wholesale_not_truncated() {
    // Truncation would silently hide fleet members and let the speaker
    // choose which 64 survive; refusal keeps the last well-formed statement
    // this agent accepted.
    let t = Tmp::new("oversize");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(voucher, vec![claim(nid(7), "seven")]);

    let flood: Vec<VouchedPeer> = (0..=atlasctl_protocol::fleet::MAX_VOUCHED)
        .map(|i| {
            let mut b = [0xB0u8; 32];
            b[31] = u8::try_from(i).expect("fits");
            b[30] = u8::try_from(i / 256).expect("fits");
            claim(NodeId::from_bytes(b), "flood")
        })
        .collect();
    assert!(flood.len() > atlasctl_protocol::fleet::MAX_VOUCHED);
    f.record_vouches(voucher, flood.clone());

    for entry in &flood {
        assert!(
            row(&f, entry.node).is_none(),
            "no entry of a refused digest may be recorded"
        );
    }
    assert!(
        row(&f, nid(7)).is_some(),
        "the previous well-formed digest remains the accepted statement"
    );
}

#[test]
fn the_voucher_choice_is_deterministic_and_prefers_the_better_leg() {
    let t = Tmp::new("tiebreak");
    let f = fleet_at(&t.0);
    let target = nid(9);
    let (a, b) = (nid(2), nid(5));
    f.record_report(report_over(a, LinkClass::Ethernet));
    f.record_report(report_over(b, LinkClass::Roce));
    f.record_vouches(a, vec![claim(target, "t")]);
    f.record_vouches(b, vec![claim(target, "t")]);

    // Unequal legs: the faster leg FROM US wins, whatever the id order says.
    assert_eq!(f.choose_voucher(target), Some(b));

    // Equal legs: the numerically smallest voucher id, and the same answer
    // twice — an unstable choice would make the UI reorder itself and a
    // failure unreproducible.
    f.record_report(report_over(a, LinkClass::Roce));
    assert_eq!(f.choose_voucher(target), Some(a));
    assert_eq!(f.choose_voucher(target), Some(a));

    // The listing's `reached_via` is the router's own choice, not a second
    // opinion: one function decides both (SSOT).
    let node = row(&f, target).expect("listed");
    assert_eq!(node.reached_via, f.choose_voucher(target));

    // A voucher we cannot currently hear is ineligible however good its leg
    // was: a route through a silent relay is not a route.
    f.clear_report(a);
    assert_eq!(f.choose_voucher(target), Some(b));
}

#[test]
fn silencing_or_unpairing_a_voucher_takes_its_claims_with_it() {
    let t = Tmp::new("revoke");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(voucher, vec![claim(nid(7), "seven")]);
    assert!(row(&f, nid(7)).is_some());

    // A voucher that stops answering stops steering: its claims cannot be
    // re-confirmed, so they must not keep populating routes and rows.
    f.clear_report(voucher);
    assert!(row(&f, nid(7)).is_none());
    assert_eq!(f.choose_voucher(nid(7)), None);

    // Unpairing does the same through the fleet surface.
    f.record_report(report_over(voucher, LinkClass::Roce));
    f.record_vouches(voucher, vec![claim(nid(7), "seven")]);
    assert!(row(&f, nid(7)).is_some());
    let _ = f.unpair(voucher);
    assert!(
        row(&f, nid(7)).is_none(),
        "trust withdrawn from a voucher withdraws every claim it made"
    );
}

#[test]
fn second_hand_vitals_appear_only_with_a_fresh_stated_age() {
    let t = Tmp::new("vitalsage");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    f.record_report(report_over(voucher, LinkClass::Roce));

    // Fresh, with an age: shown.
    let mut fresh = claim(nid(7), "fresh");
    fresh.vitals = Some(vitals());
    fresh.vitals_age_s = Some(10);
    // Ancient: the voucher last heard these before its own staleness bound.
    let mut stale = claim(nid(8), "stale");
    stale.vitals = Some(vitals());
    stale.vitals_age_s = Some(UNREACHABLE_AFTER.as_secs() + 1);
    // Ageless: an unknown age is not zero; showing these would render old
    // data as current.
    let mut ageless = claim(nid(9), "ageless");
    ageless.vitals = Some(vitals());
    ageless.vitals_age_s = None;
    f.record_vouches(voucher, vec![fresh, stale, ageless]);

    assert!(row(&f, nid(7)).expect("listed").vitals.is_some());
    assert!(row(&f, nid(8)).expect("listed").vitals.is_none());
    assert!(row(&f, nid(9)).expect("listed").vitals.is_none());
}
