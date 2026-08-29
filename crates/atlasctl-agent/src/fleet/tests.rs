// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::identity::Identity;
use atlasctl_protocol::fleet::{LinkClass, Metric};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

pub(super) struct Tmp(pub(super) PathBuf);

impl Tmp {
    pub(super) fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-fleet-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&p).expect("scratch");
        Self(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn roce(addr: &str) -> NodeAddress {
    NodeAddress {
        iface: "enp1s0f0np0".to_owned(),
        addr: addr.to_owned(),
        class: LinkClass::Roce,
        speed_mbps: Some(200_000),
        prefix_len: 30,
        rdma: true,
    }
}

pub(super) fn fleet_at(dir: &Path) -> LocalFleet {
    LocalFleet::new(
        Identity::generate(),
        PinStore::new(dir),
        DisplayName::new("spark-256a"),
        vec![roce("10.10.10.9")],
        Launchability::yes(),
        "GB10".to_owned(),
    )
}

pub(super) fn beacon(id: NodeId, name: &str, can_launch: bool) -> Beacon {
    Beacon {
        id,
        name: DisplayName::new(name),
        peer_port: 34334,
        addresses: vec!["10.10.10.10".parse::<IpAddr>().expect("addr")],
        can_launch,
        accelerator: "GB10".to_owned(),
    }
}

#[test]
fn a_lone_agent_reports_itself_and_nothing_else() {
    let t = Tmp::new("solo");
    let f = fleet_at(&t.0);
    let nodes = f.nodes();
    assert_eq!(nodes.len(), 1);
    assert!(nodes[0].is_local);
    assert_eq!(nodes[0].pairing, PairingState::Paired);
    assert_eq!(nodes[0].addresses[0].class, LinkClass::Roce);
}

#[test]
fn a_discovered_node_is_visible_trusted_by_nobody_and_reports_no_vitals() {
    // The whole point of separating discovery from trust. Anything a stranger
    // broadcasts must be renderable without being believable.
    let t = Tmp::new("disc");
    let f = fleet_at(&t.0);
    f.observe(beacon(NodeId::from_bytes([7; 32]), "spark-43fa", true));

    let peer = f
        .nodes()
        .into_iter()
        .find(|n| !n.is_local)
        .expect("the beacon is listed");
    assert_eq!(peer.pairing, PairingState::Discovered);
    assert!(
        peer.vitals.is_none(),
        "telemetry from an unpaired machine is not evidence about your fleet"
    );
    assert!(peer.alerts.is_empty());
}

#[test]
fn a_beacon_cannot_write_itself_into_the_pin_store() {
    let t = Tmp::new("nopin");
    let f = fleet_at(&t.0);
    let stranger = NodeId::from_bytes([9; 32]);
    f.observe(beacon(stranger, "evil", true));
    assert!(
        !PinStore::new(&t.0).is_pinned(stranger).expect("reads"),
        "observing a beacon must never grant trust"
    );
}

#[test]
fn an_agent_ignores_its_own_beacon() {
    // Every agent hears itself on the multicast group. Listing yourself twice
    // would double every fleet and make the local node look like a peer.
    let t = Tmp::new("self");
    let f = fleet_at(&t.0);
    f.observe(beacon(f.id(), "spark-256a", true));
    assert_eq!(f.nodes().len(), 1);
}

#[test]
fn a_paired_node_that_is_switched_off_stays_in_the_fleet_as_unreachable() {
    // A fleet that forgets its members when they sleep is not a fleet, and a
    // node vanishing is indistinguishable from a node being removed.
    let t = Tmp::new("off");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("spark-43fa"),
        1_756_000_000,
        None,
        false,
    )
    .expect("pin");

    let listed = f
        .nodes()
        .into_iter()
        .find(|n| n.id == peer.id())
        .expect("a pinned peer is listed even with no sighting");
    assert_eq!(listed.pairing, PairingState::Unreachable);
    assert!(!listed.launchability.can_launch);
    assert!(
        listed.launchability.reason.contains("reachable"),
        "the reason has to say what is wrong: {}",
        listed.launchability.reason
    );
}

#[test]
fn a_paired_node_that_is_answering_is_reported_as_paired() {
    let t = Tmp::new("on");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("spark-43fa"),
        0,
        None,
        false,
    )
    .expect("pin");
    f.observe(beacon(peer.id(), "spark-43fa", true));

    let listed = f
        .nodes()
        .into_iter()
        .find(|n| n.id == peer.id())
        .expect("listed");
    assert_eq!(listed.pairing, PairingState::Paired);
    assert!(listed.launchability.can_launch);
    // Still no vitals: those come over the authenticated peer channel, not from
    // a beacon.
    assert!(listed.vitals.is_none());
}

#[test]
fn a_node_is_listed_once_even_when_pinned_and_seen() {
    let t = Tmp::new("dupe");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("p"),
        0,
        None,
        false,
    )
    .expect("pin");
    f.observe(beacon(peer.id(), "p", true));
    assert_eq!(f.nodes().iter().filter(|n| n.id == peer.id()).count(), 1);
}

#[test]
fn a_beacon_claiming_it_cannot_launch_is_taken_at_its_word() {
    let t = Tmp::new("cl");
    let f = fleet_at(&t.0);
    f.observe(beacon(NodeId::from_bytes([3; 32]), "laptop", false));
    let peer = f.nodes().into_iter().find(|n| !n.is_local).expect("listed");
    assert!(!peer.launchability.can_launch);
}

/// OS travels on the authenticated channel and nowhere else. A machine we
/// have only *seen* has told us nothing about itself we can believe, and the
/// interface must show a blank rather than a guess.
#[test]
fn a_discovered_node_reports_no_operating_system() {
    let t = Tmp::new("osdisc");
    let f = fleet_at(&t.0);
    f.observe(beacon(NodeId::from_bytes([4; 32]), "stranger", true));
    let peer = f.nodes().into_iter().find(|n| !n.is_local).expect("listed");
    assert_eq!(peer.os, "", "a beacon must not be able to claim an OS");
}

/// And this machine does report its own, because it is the one thing here we
/// know first-hand.
#[test]
fn the_local_node_reports_its_operating_system() {
    let t = Tmp::new("oslocal");
    let f = fleet_at(&t.0);
    let me = f.nodes().into_iter().find(|n| n.is_local).expect("local");
    assert!(!me.os.is_empty(), "this machine knows what it is running");
}

fn report(id: NodeId, name: &str, can_launch: bool) -> crate::peer::link::PeerReport {
    crate::peer::link::PeerReport {
        node: id,
        name: name.to_owned(),
        can_launch,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link: LinkClass::Roce,
        addresses: vec![roce("10.10.10.10")],
        vouched: None,
        peer_version_max: crate::peer::wire::PEER_PROTOCOL_VERSION,
    }
}

/// Enterprise wifi filters multicast and the Spark links are point-to-point
/// /30s, so a paired machine having no beacon is ordinary — it is the case
/// `peer add` exists for. Launchability was read from the beacon alone, so
/// such a machine reported "not reachable right now" and could not be given a
/// rank, moments after completing an authenticated handshake with us.
#[test]
fn a_paired_peer_we_have_spoken_to_is_launchable_without_a_beacon() {
    let t = Tmp::new("noboacon");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("spark-43fa"),
        0,
        None,
        false,
    )
    .expect("pin");
    // No `observe`: nothing was ever heard on multicast.
    f.record_report(report(peer.id(), "spark-43fa", true));

    let listed = f
        .nodes()
        .into_iter()
        .find(|n| n.id == peer.id())
        .expect("listed");
    assert_eq!(listed.pairing, PairingState::Paired);
    assert!(
        listed.launchability.can_launch,
        "a peer we authenticated with was called unreachable: {:?}",
        listed.launchability.reason
    );
}

/// The authenticated channel outranks the beacon, and it has to: a beacon is
/// unauthenticated, so believing it over something that proved it holds the
/// pinned key would let anyone on the network decide whether a machine of ours
/// is allowed to hold a rank.
#[test]
fn an_authenticated_report_outranks_a_beacon_that_disagrees() {
    let t = Tmp::new("outrank");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("laptop"),
        0,
        None,
        false,
    )
    .expect("pin");
    // The beacon says it can launch; the machine itself says it cannot.
    f.observe(beacon(peer.id(), "laptop", true));
    f.record_report(report(peer.id(), "laptop", false));

    let listed = f
        .nodes()
        .into_iter()
        .find(|n| n.id == peer.id())
        .expect("listed");
    assert!(
        !listed.launchability.can_launch,
        "a beacon overrode the machine's own authenticated answer"
    );
}

#[tokio::test]
async fn pairing_refuses_a_code_of_the_wrong_shape_without_touching_the_network() {
    let t = Tmp::new("shape");
    let f = fleet_at(&t.0);
    let node = NodeId::from_bytes([5; 32]);
    f.observe(beacon(node, "spark-43fa", true));
    let err = f
        .pair(node, "12")
        .await
        .expect_err("a two-digit code is not a code");
    assert!(err.to_string().contains("digits"));
}

#[tokio::test]
async fn pairing_a_node_that_was_never_seen_is_refused() {
    let t = Tmp::new("unseen");
    let f = fleet_at(&t.0);
    let err = f
        .pair(NodeId::from_bytes([4; 32]), "12345678")
        .await
        .expect_err("cannot pair with something that is not there");
    assert!(err.to_string().contains("not visible"));
}

#[test]
fn unpairing_reports_whether_there_was_anything_to_undo() {
    let t = Tmp::new("unpair");
    let f = fleet_at(&t.0);
    let peer = Identity::generate();
    record_pairing(
        &PinStore::new(&t.0),
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("p"),
        0,
        None,
        false,
    )
    .expect("pin");

    assert!(f.unpair(peer.id()).expect("removes"));
    assert!(!f.unpair(peer.id()).expect("second time is a no-op"));
}

#[test]
fn pruning_keeps_paired_nodes_and_drops_stale_strangers() {
    let t = Tmp::new("prune");
    let f = fleet_at(&t.0);
    let stranger = NodeId::from_bytes([8; 32]);
    f.observe(beacon(stranger, "stranger", true));
    // Fresh sighting survives.
    f.prune();
    assert!(f.nodes().iter().any(|n| n.id == stranger));
}

struct FixedVitals(NodeVitals);

impl VitalsSource for FixedVitals {
    fn vitals(&self) -> NodeVitals {
        self.0.clone()
    }
}

#[test]
fn the_local_node_carries_whatever_vitals_the_machine_can_answer() {
    let t = Tmp::new("vitals");
    let f = fleet_at(&t.0).with_vitals(Box::new(FixedVitals(NodeVitals {
        accelerator_util: Metric::reading(96.0),
        memory_total_bytes: Metric::Unsupported,
        ..NodeVitals::default()
    })));
    let local = f.nodes().remove(0);
    let v = local.vitals.expect("the local node reports vitals");
    assert_eq!(v.accelerator_util, Metric::reading(96.0));
    assert_eq!(v.memory_total_bytes, Metric::Unsupported);
}

#[test]
fn an_unanswerable_device_field_becomes_unsupported_not_zero() {
    // The GB10 case, at the conversion boundary: nvidia-smi answers N/A for
    // memory because Grace-Blackwell has no framebuffer.
    use atlasctl_protocol::telemetry::DeviceStats;
    let d = DeviceStats {
        gpu_util_pct: Some(96.0),
        sm_clock_mhz: Some(2405),
        memory_total_bytes: None,
        memory_used_bytes: None,
        ..DeviceStats::default()
    };
    let v = vitals_from_device(&d, None, true, 10, Some(1500));
    assert_eq!(v.accelerator_util, Metric::reading(96.0));
    assert_eq!(v.memory_total_bytes, Metric::Unsupported);
    assert_eq!(v.memory_used_frac, Metric::Unsupported);
    assert_eq!(v.disk_free_bytes, Metric::Unsupported);
    assert_ne!(v.memory_total_bytes, Metric::reading(0.0));
}

#[test]
fn a_paired_peers_address_survives_this_agent_restarting() {
    // Sightings live in memory; pins live on disk. Without remembering the
    // address, restarting the agent made a paired machine that was up and
    // answering render as "no usable network link" until mDNS happened to
    // re-announce it — up to a minute on a quiet network, and indistinguishable
    // from a peer with no fabric at all.
    let t = Tmp::new("addrmem");
    let peer = Identity::generate();
    let pins = PinStore::new(&t.0);
    // The address is recorded by the PAIRING — `peer add` pins the address the
    // ceremony actually completed over — and refreshed by an authenticated
    // poll. It used to be written here by `observe`, from an unauthenticated
    // beacon; see `a_beacon_cannot_move_a_pinned_peer` for why it is not.
    record_pairing(
        &pins,
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("spark-43fa"),
        0,
        Some("10.10.10.10".to_owned()),
        false,
    )
    .expect("pin");

    // A fresh process, no sightings at all.
    let restarted = fleet_at(&t.0);
    let listed = restarted
        .nodes()
        .into_iter()
        .find(|n| n.id == peer.id())
        .expect("a pinned peer is still listed");
    assert_eq!(
        listed.addresses.first().map(|a| a.addr.as_str()),
        Some("10.10.10.10"),
        "a restart must not forget where a paired machine lives"
    );
    // Remembered, not re-verified: the class stays unverified until the peer
    // says so over the authenticated channel.
    assert_eq!(
        listed.addresses[0].class,
        atlasctl_protocol::fleet::LinkClass::Unverified
    );
}

#[test]
fn an_unverified_link_is_usable_but_never_preferred_and_never_warns() {
    use atlasctl_protocol::fleet::LinkClass;
    // It is an absence of information. Treating it as a problem would invent
    // one; treating it as a preference would let an unauthenticated beacon
    // outrank a link we measured ourselves.
    assert!(LinkClass::Unverified.usable_for_cluster());
    assert!(!LinkClass::Unverified.warns());
    assert!(LinkClass::Unverified.rank() < LinkClass::Ethernet.rank());
    assert!(LinkClass::Unverified.rank() > LinkClass::Virtual.rank());
}
