// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn fp(byte: u8) -> NodeId {
    NodeId::from_bytes([byte; 32])
}

#[test]
fn a_beacon_round_trips_through_its_txt_record() {
    let b = Beacon {
        id: fp(0xa3),
        name: DisplayName::new("spark-256a"),
        peer_port: 34334,
        addresses: vec!["10.10.10.9".parse().expect("addr")],
        can_launch: true,
        accelerator: "GB10".to_owned(),
    };
    let props = b.txt_properties();
    let back = Beacon::from_txt(&props, b.addresses.clone(), b.peer_port).expect("parses");
    assert_eq!(back, b);
}

#[test]
fn a_beacon_carries_no_version_no_inventory_and_not_the_browser_port() {
    // Everything published here is unauthenticated and visible to the whole
    // network, forever. A version string would let someone scan for a known
    // vulnerable release; the browser port is loopback-only and has no business
    // being advertised at all.
    let b = Beacon {
        id: fp(1),
        name: DisplayName::new("spark-256a"),
        peer_port: 34334,
        addresses: vec![],
        can_launch: true,
        accelerator: "GB10".to_owned(),
    };
    let keys: Vec<String> = b.txt_properties().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, ["id", "name", "cl", "gpu"]);

    let flat = format!("{:?}", b.txt_properties());
    assert!(
        !flat.contains("34333"),
        "the browser control port must never be advertised"
    );
    assert!(
        !flat.contains(env!("CARGO_PKG_VERSION")),
        "no version in a beacon"
    );
}

#[test]
fn a_record_without_a_usable_fingerprint_is_ignored_not_an_error() {
    // Malformed records are ambient noise on a shared network.
    let addrs = vec!["10.0.0.1".parse().expect("addr")];
    assert!(Beacon::from_txt(&[], addrs.clone(), 34334).is_none());
    assert!(
        Beacon::from_txt(
            &[("id".to_owned(), "not-a-fingerprint".to_owned())],
            addrs.clone(),
            34334
        )
        .is_none()
    );
    // Right shape, wrong alphabet.
    assert!(Beacon::from_txt(&[("id".to_owned(), "z".repeat(64))], addrs, 34334).is_none());
}

#[test]
fn hostile_strings_in_a_beacon_are_sanitised_before_anything_renders_them() {
    let props = vec![
        ("id".to_owned(), fp(2).to_string()),
        ("name".to_owned(), "\u{1b}[31mspark\u{0}\n".to_owned()),
        ("gpu".to_owned(), "A".repeat(500)),
    ];
    let b = Beacon::from_txt(&props, vec![], 34334).expect("parses");
    assert!(!b.name.as_str().contains('\u{1b}'));
    assert!(!b.name.as_str().contains('\u{0}'));
    assert!(
        b.accelerator.len() <= 63,
        "an unbounded field would be a UI denial of service"
    );
}

#[test]
fn can_launch_defaults_to_false_when_the_beacon_does_not_say() {
    // A claim that is absent must not read as a capability.
    let props = vec![("id".to_owned(), fp(3).to_string())];
    let b = Beacon::from_txt(&props, vec![], 34334).expect("parses");
    assert!(!b.can_launch);
}

#[test]
fn discovery_can_be_switched_off_entirely() {
    let n = NoDiscovery;
    let b = Beacon {
        id: fp(4),
        name: DisplayName::new("x"),
        peer_port: 34334,
        addresses: vec![],
        can_launch: false,
        accelerator: String::new(),
    };
    n.advertise(&b).expect("no-op");
    n.withdraw().expect("no-op");
    // The channel must be closed, not merely empty, or a caller that loops
    // over it hangs forever in the hardened preset.
    let rx = n.browse().expect("no-op browse");
    assert!(rx.recv().is_err(), "an off browser yields a closed channel");
}

#[test]
fn a_manually_typed_peer_resolves_with_and_without_a_port() {
    let with = resolve_manual("127.0.0.1:9999", 34334).expect("resolves");
    assert_eq!(with[0].port(), 9999);
    let without = resolve_manual("127.0.0.1", 34334).expect("resolves");
    assert_eq!(without[0].port(), 34334);
    assert!(resolve_manual("no-such-host.invalid", 34334).is_err());
}

/// The TXT record is an unauthenticated broadcast. It deliberately carries no
/// operating system, no agent version and no recipe inventory: those are a
/// shopping list for anyone listening, and none of them is needed to draw a
/// grey node on a graph.
#[test]
fn the_beacon_advertises_nothing_a_stranger_could_shop_from() {
    let b = Beacon {
        id: NodeId::from_bytes([1; 32]),
        name: DisplayName::new("spark-256a"),
        peer_port: 34334,
        addresses: Vec::new(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
    };
    let txt = b.txt_properties();
    let keys: Vec<&str> = txt.iter().map(|(k, _)| k.as_str()).collect();
    for forbidden in ["os", "version", "agent_version", "recipes", "browser_port"] {
        assert!(
            !keys.contains(&forbidden),
            "the beacon must not carry {forbidden}: {keys:?}"
        );
    }
}
