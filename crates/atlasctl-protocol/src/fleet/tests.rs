// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

const FP: &str = "3f2a1b0c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8";

#[test]
fn a_node_id_round_trips_through_its_hex_form() {
    let id = NodeId::parse(FP).expect("fixture is valid hex");
    assert_eq!(id.to_string(), FP);
    assert_eq!(NodeId::parse(&id.to_string()).expect("round trip"), id);
}

#[test]
fn a_node_id_refuses_anything_that_is_not_a_fingerprint() {
    assert!(matches!(NodeId::parse(""), Err(NodeIdError::Length(0))));
    assert!(matches!(
        NodeId::parse(&FP[..63]),
        Err(NodeIdError::Length(63))
    ));
    // Right length, wrong alphabet: the 'g' must be rejected, not silently
    // treated as zero.
    let bad = format!("g{}", &FP[1..]);
    assert!(matches!(NodeId::parse(&bad), Err(NodeIdError::NotHex(0))));
}

#[test]
fn the_short_form_is_what_a_human_reads_off_a_screen() {
    let id = NodeId::parse(FP).expect("fixture");
    assert_eq!(id.short(), "3f2a-1b0c-4d5e-6f70");
}

#[test]
fn a_display_name_cannot_carry_control_characters_or_run_long() {
    // A beacon is an unauthenticated write into someone's UI. Newlines and
    // escapes must not survive to the renderer.
    let hostile = DisplayName::new("spark\u{0}-\u{1b}[31mred\n\n");
    assert!(!hostile.as_str().contains('\u{1b}'));
    assert!(!hostile.as_str().contains('\u{0}'));
    assert!(!hostile.as_str().contains('\n'));

    let long = DisplayName::new(&"a".repeat(500));
    assert_eq!(long.as_str().len(), DISPLAY_MAX);

    // Something that sanitises to nothing still has to render as something.
    assert_eq!(DisplayName::new("   \u{0}  ").as_str(), "unnamed");
}

#[test]
fn link_preference_puts_the_fabric_the_numbers_were_measured_on_first() {
    let mut classes = [
        LinkClass::Ethernet,
        LinkClass::Virtual,
        LinkClass::InfiniBand,
        LinkClass::Wireless,
        LinkClass::Roce,
    ];
    classes.sort_by_key(|c| std::cmp::Reverse(c.rank()));
    assert_eq!(
        classes,
        [
            LinkClass::InfiniBand,
            LinkClass::Roce,
            LinkClass::Ethernet,
            LinkClass::Wireless,
            LinkClass::Virtual
        ]
    );
    // Anything that is not RDMA-backed must warn: EP=2 decode is all-reduce
    // bound, so ethernet is a silent multiple-x loss, not a small one.
    assert!(!LinkClass::Roce.warns());
    assert!(!LinkClass::InfiniBand.warns());
    assert!(LinkClass::Ethernet.warns());
    assert!(LinkClass::Wireless.warns());
    // A docker bridge is never a cluster fabric.
    assert!(!LinkClass::Virtual.usable_for_cluster());
    assert!(!LinkClass::Loopback.usable_for_cluster());
}

#[test]
fn an_absent_measurement_is_not_a_zero() {
    // The GB10 case: nvidia-smi answers N/A for framebuffer memory because
    // there is no framebuffer. Rendering 0 there would be inventing a reading.
    let absent: Metric = None.into();
    assert_eq!(absent, Metric::Unsupported);
    assert_eq!(absent.value(), None);
    assert_ne!(absent, Metric::reading(0.0));

    // It must survive the wire as a distinct state, not as null-that-becomes-0.
    let json = serde_json::to_string(&absent).expect("serialises");
    assert_eq!(json, r#"{"state":"unsupported"}"#);
    let back: Metric = serde_json::from_str(&json).expect("round trips");
    assert_eq!(back, Metric::Unsupported);
}

fn addr(iface: &str, addr: &str, class: LinkClass, speed: Option<u32>) -> NodeAddress {
    NodeAddress {
        iface: iface.to_owned(),
        addr: addr.to_owned(),
        class,
        speed_mbps: speed,
        rdma: matches!(class, LinkClass::Roce | LinkClass::InfiniBand),
        // Point-to-point, like the RoCE links on a real Spark.
        prefix_len: 30,
    }
}

fn node(addresses: Vec<NodeAddress>) -> NodeDescriptor {
    NodeDescriptor {
        id: NodeId::parse(FP).expect("fixture"),
        name: DisplayName::new("spark-256a"),
        is_local: false,
        pairing: PairingState::Paired,
        addresses,
        launchability: Launchability::yes(),
        agent_version: "0.1.2".to_owned(),
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        alerts: Vec::new(),
        running: None,
    }
}

#[test]
fn the_preferred_address_is_the_roce_link_not_the_docker_bridge() {
    // This is the real interface table on a DGX Spark, in the order the kernel
    // happens to list it. Picking the first usable address would pick a docker
    // bridge and quietly run the collective over a software switch.
    let n = node(vec![
        addr("docker0", "172.17.0.1", LinkClass::Virtual, None),
        addr(
            "docker_gwbridge",
            "172.19.0.1",
            LinkClass::Virtual,
            Some(10000),
        ),
        addr("wlP9s9", "192.168.68.68", LinkClass::Wireless, None),
        addr("dummy0", "10.10.10.1", LinkClass::Virtual, None),
        addr("enp1s0f0np0", "10.10.10.9", LinkClass::Roce, Some(200_000)),
    ]);
    let best = n.preferred_address().expect("one usable address");
    assert_eq!(best.iface, "enp1s0f0np0");
    assert_eq!(best.class, LinkClass::Roce);
}

#[test]
fn between_two_links_of_one_class_the_faster_one_wins() {
    let n = node(vec![
        addr("eth0", "10.0.0.1", LinkClass::Ethernet, Some(1000)),
        addr("eth1", "10.0.1.1", LinkClass::Ethernet, Some(25000)),
    ]);
    assert_eq!(n.preferred_address().expect("usable").iface, "eth1");
}

#[test]
fn a_node_with_only_virtual_links_has_no_usable_address() {
    let n = node(vec![
        addr("docker0", "172.17.0.1", LinkClass::Virtual, None),
        addr("lo", "127.0.0.1", LinkClass::Loopback, None),
    ]);
    assert!(n.preferred_address().is_none());
}

#[test]
fn a_control_only_node_says_why_it_cannot_launch() {
    let l = Launchability::no("this agent runs in --client mode");
    assert!(!l.can_launch);
    assert!(
        !l.reason.is_empty(),
        "a refusal without a reason is not actionable"
    );
}

/// A join invitation names an address the other machine will DIAL. The
/// predicate for that is reachability, not whether the link could carry a
/// collective — and those differ on exactly one class.
#[test]
fn wireless_can_be_dialed_even_though_it_cannot_carry_a_collective() {
    assert!(
        LinkClass::Wireless.usable_for_control(),
        "a laptop on Wi-Fi is the canonical inviter; excluding it left the \
         invitation with no address at all"
    );
    assert!(!LinkClass::Wireless.usable_for_cluster());
}

/// The two predicates must not drift into being the same thing, and must agree
/// about what is unreachable.
#[test]
fn only_links_reachable_from_another_machine_are_dialable() {
    for c in [
        LinkClass::InfiniBand,
        LinkClass::Roce,
        LinkClass::Ethernet,
        LinkClass::Wireless,
        LinkClass::Unverified,
    ] {
        assert!(c.usable_for_control(), "{c:?} should be dialable");
    }
    for c in [LinkClass::Loopback, LinkClass::Virtual] {
        assert!(!c.usable_for_control(), "{c:?} is reachable from nowhere else");
    }
}
