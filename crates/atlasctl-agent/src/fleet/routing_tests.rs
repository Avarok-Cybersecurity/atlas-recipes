// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for control routing: where a request would go, and — the part the
//! trust model rides on — where it must never go.

use super::routing::ControlRoute;
use super::tests::{Tmp, fleet_at};
use super::{LocalFleet, record_pairing};
use atlasctl_protocol::fleet::{DisplayName, LinkClass, NodeAddress, NodeId, VouchedPeer};
use atlasctl_protocol::msg::AgentError;

const PORT: u16 = 34334;

fn nid(n: u8) -> NodeId {
    NodeId::from_bytes([n; 32])
}

fn addr_of(iface_addr: &str, class: LinkClass, speed: Option<u32>) -> NodeAddress {
    NodeAddress {
        iface: String::new(),
        addr: iface_addr.to_owned(),
        class,
        speed_mbps: speed,
        prefix_len: 24,
        rdma: matches!(class, LinkClass::Roce | LinkClass::InfiniBand),
    }
}

fn report(
    id: NodeId,
    addresses: Vec<NodeAddress>,
    peer_version_max: u32,
) -> crate::peer::link::PeerReport {
    crate::peer::link::PeerReport {
        node: id,
        name: "peer".to_owned(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link: LinkClass::Roce,
        addresses,
        vouched: None,
        peer_version_max,
    }
}

fn claim(node: NodeId, addresses: Vec<NodeAddress>) -> VouchedPeer {
    VouchedPeer {
        node,
        name: DisplayName::new("vouched"),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        addresses,
        link: LinkClass::Roce,
        reachable: true,
        vitals: None,
        vitals_age_s: None,
    }
}

fn pin(f: &LocalFleet, node: NodeId, last_address: Option<&str>) {
    record_pairing(
        &f.pins,
        node,
        "aa",
        DisplayName::new("pinned"),
        0,
        last_address.map(ToOwned::to_owned),
        false,
    )
    .expect("pin");
}

#[test]
fn a_pinned_peer_is_dialled_directly_on_its_best_reported_link() {
    let t = Tmp::new("direct");
    let f = fleet_at(&t.0);
    let target = nid(7);
    pin(&f, target, Some("192.168.1.7"));
    // The report offers LAN and fabric; the fabric must win — this is the
    // InfiniBand-preference requirement falling out of the shared
    // max_by_key((rank, speed)) selection.
    f.record_report(report(
        target,
        vec![
            addr_of("192.168.1.7", LinkClass::Ethernet, Some(1_000)),
            addr_of("10.10.10.7", LinkClass::Roce, Some(200_000)),
        ],
        2,
    ));

    match f.plan_control_route(target, PORT).expect("routable") {
        ControlRoute::Direct { addr } => {
            assert_eq!(addr.to_string(), format!("10.10.10.7:{PORT}"));
        }
        other @ ControlRoute::Via { .. } => panic!("a pinned peer must not be relayed: {other:?}"),
    }
}

#[test]
fn a_pinned_peer_never_heard_from_falls_back_to_its_remembered_address() {
    let t = Tmp::new("lastaddr");
    let f = fleet_at(&t.0);
    let target = nid(7);
    pin(&f, target, Some("192.168.1.7"));

    match f.plan_control_route(target, PORT).expect("routable") {
        ControlRoute::Direct { addr } => {
            assert_eq!(addr.to_string(), format!("192.168.1.7:{PORT}"));
        }
        other @ ControlRoute::Via { .. } => panic!("a pinned peer must not be relayed: {other:?}"),
    }
}

#[test]
fn a_vouched_node_is_reached_only_through_its_voucher_never_its_claimed_address() {
    // The lying-intermediary-address case, at the planning layer: the digest
    // carries a hostile address for the target, and the plan must route to
    // the VOUCHER's own dialable address instead — the claimed address
    // contributes zero bytes.
    let t = Tmp::new("viavoucher");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    let target = nid(7);
    pin(&f, voucher, None);
    f.record_report(report(
        voucher,
        vec![addr_of("10.10.10.3", LinkClass::Roce, Some(200_000))],
        2,
    ));
    f.record_vouches(
        voucher,
        vec![claim(
            target,
            vec![addr_of("6.6.6.6", LinkClass::Roce, Some(200_000))],
        )],
    );

    match f.plan_control_route(target, PORT).expect("routable") {
        ControlRoute::Via { relay, addr } => {
            assert_eq!(relay, voucher);
            assert_eq!(
                addr.to_string(),
                format!("10.10.10.3:{PORT}"),
                "the dial must go to the voucher, at OUR address for it"
            );
        }
        other @ ControlRoute::Direct { .. } => {
            panic!("a vouched-only node must never be dialled directly: {other:?}")
        }
    }
    assert!(
        f.control_address(target).is_none(),
        "a digest's claimed address must never become a dialable address"
    );
}

#[test]
fn no_pin_and_no_vouch_is_not_routable_and_says_to_pair() {
    let t = Tmp::new("nowhere");
    let f = fleet_at(&t.0);
    match f.plan_control_route(nid(9), PORT) {
        Err(AgentError::NotRoutable { node, reason }) => {
            assert_eq!(node, nid(9));
            assert!(reason.contains("pair"), "actionable fix, got: {reason}");
        }
        other => panic!("expected NotRoutable, got {other:?}"),
    }
}

#[test]
fn a_silent_voucher_makes_its_nodes_not_routable_with_provenance() {
    // The voucher claimed the target, then stopped answering: the row stays
    // listed (fleet), but the ROUTE is gone, and the refusal names the
    // machine to wake rather than the one to pair with.
    let t = Tmp::new("silentvoucher");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    let target = nid(7);
    pin(&f, voucher, None);
    f.record_report(report(voucher, Vec::new(), 2));
    f.record_vouches(voucher, vec![claim(target, Vec::new())]);
    f.clear_report(voucher);

    match f.plan_control_route(target, PORT) {
        Err(AgentError::NotRoutable { reason, .. }) => {
            assert!(
                reason.contains(&voucher.short()),
                "must name the voucher to wake, got: {reason}"
            );
        }
        other => panic!("expected NotRoutable, got {other:?}"),
    }
}

#[test]
fn unpairing_the_voucher_kills_every_route_through_it() {
    let t = Tmp::new("unpairvoucher");
    let f = fleet_at(&t.0);
    let voucher = nid(3);
    let target = nid(7);
    pin(&f, voucher, None);
    f.record_report(report(
        voucher,
        vec![addr_of("10.10.10.3", LinkClass::Roce, None)],
        2,
    ));
    f.record_vouches(voucher, vec![claim(target, Vec::new())]);
    assert!(f.plan_control_route(target, PORT).is_ok());

    use crate::fleet::FleetView as _;
    f.unpair(voucher).expect("unpair");
    f.clear_report(voucher);
    assert!(
        matches!(
            f.plan_control_route(target, PORT),
            Err(AgentError::NotRoutable { .. })
        ),
        "trust withdrawn from the voucher must withdraw the routes it carried"
    );
}

#[test]
fn a_v1_next_hop_is_refused_locally_naming_the_version() {
    // O5, both shapes of next hop. The refusal happens before any dial: a v2
    // frame at a v1 build is dropped by its decoder, which would read as the
    // network eating the request.
    let t = Tmp::new("v1hop");
    let f = fleet_at(&t.0);

    // Direct: the pinned target itself is old.
    let target = nid(7);
    pin(&f, target, Some("192.168.1.7"));
    f.record_report(report(target, Vec::new(), 1));
    match f.plan_control_route(target, PORT) {
        Err(AgentError::NotRoutable { reason, .. }) => {
            assert!(reason.contains("protocol 1"), "got: {reason}");
        }
        other => panic!("expected a version refusal, got {other:?}"),
    }

    // Via: the voucher is old.
    let voucher = nid(3);
    let vouched = nid(9);
    pin(&f, voucher, None);
    f.record_report(report(
        voucher,
        vec![addr_of("10.10.10.3", LinkClass::Roce, None)],
        1,
    ));
    f.record_vouches(voucher, vec![claim(vouched, Vec::new())]);
    match f.plan_control_route(vouched, PORT) {
        Err(AgentError::NotRoutable { reason, .. }) => {
            assert!(reason.contains("protocol 1"), "got: {reason}");
        }
        other => panic!("expected a version refusal, got {other:?}"),
    }
}

#[test]
fn the_listing_shows_exactly_the_route_the_planner_would_take() {
    // SSOT: `reached_via` in the listing and the planner's relay come from
    // the same choose_voucher call, and this is the assertion that keeps a
    // refactor from splitting them.
    let t = Tmp::new("ssotroute");
    let f = fleet_at(&t.0);
    let (slow, fast, target) = (nid(2), nid(4), nid(7));
    for v in [slow, fast] {
        pin(&f, v, None);
        f.record_vouches(v, vec![claim(target, Vec::new())]);
    }
    f.record_report(report(
        slow,
        vec![addr_of("192.168.1.2", LinkClass::Ethernet, None)],
        2,
    ));
    let mut fast_report = report(fast, vec![addr_of("10.10.10.4", LinkClass::Roce, None)], 2);
    fast_report.link = LinkClass::Roce;
    let mut slow_report = report(
        slow,
        vec![addr_of("192.168.1.2", LinkClass::Ethernet, None)],
        2,
    );
    slow_report.link = LinkClass::Ethernet;
    f.record_report(slow_report);
    f.record_report(fast_report);

    let planned = match f.plan_control_route(target, PORT).expect("routable") {
        ControlRoute::Via { relay, .. } => relay,
        ControlRoute::Direct { .. } => panic!("target is not pinned"),
    };
    use crate::fleet::FleetView as _;
    let listed = f
        .nodes()
        .into_iter()
        .find(|n| n.id == target)
        .expect("listed")
        .reached_via;
    assert_eq!(
        listed,
        Some(planned),
        "the UI must not show a route the router would not take"
    );
    assert_eq!(planned, fast, "the better leg from us wins");
}
