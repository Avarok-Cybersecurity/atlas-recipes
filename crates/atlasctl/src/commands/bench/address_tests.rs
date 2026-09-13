// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_agent::discovery::Beacon;
use atlasctl_protocol::fleet::{DisplayName, NodeId};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::sync::mpsc::Receiver;

#[test]
fn every_typed_form_splits_and_a_missing_port_is_the_peer_port() {
    let cases: &[(&str, &str, u16)] = &[
        ("10.10.10.2", "10.10.10.2", DEFAULT_PEER_PORT),
        ("10.10.10.2:4000", "10.10.10.2", 4000),
        ("dgx3.local", "dgx3.local", DEFAULT_PEER_PORT),
        ("dgx3.local:34334", "dgx3.local", 34334),
        ("bench.example.com", "bench.example.com", DEFAULT_PEER_PORT),
        ("bench.example.com:1", "bench.example.com", 1),
        ("fe80::1", "fe80::1", DEFAULT_PEER_PORT),
        ("[fe80::1]", "fe80::1", DEFAULT_PEER_PORT),
        ("[fe80::1]:5000", "fe80::1", 5000),
        ("  dgx2  ", "dgx2", DEFAULT_PEER_PORT),
    ];
    for (given, host, port) in cases {
        let t = Target::parse(given).unwrap_or_else(|e| panic!("{given}: {e}"));
        assert_eq!((t.host.as_str(), t.port), (*host, *port), "{given}");
        assert_eq!(t.given, *given);
    }
}

#[test]
fn malformed_addresses_are_bad_args_before_any_dial() {
    for given in [
        "",
        "   ",
        ":34334",
        "dgx2:",
        "dgx2:abc",
        "dgx2:0",
        "dgx2:65536",
        "[fe80::1",
        "[fe80::1]x",
        "fe80::1:34334",
    ] {
        let e = Target::parse(given).expect_err(given);
        assert_eq!(e.obj.code, "bad_args", "{given}");
        assert_eq!(e.exit, super::super::exit::Code::Usage);
    }
}

#[test]
fn mdns_names_are_recognised_and_labelled() {
    let t = Target::parse("DGX2.local").unwrap();
    assert!(t.is_mdns_name());
    assert_eq!(t.mdns_label(), "dgx2");
    let t = Target::parse("dgx2.local.").unwrap();
    assert!(t.is_mdns_name());
    assert_eq!(t.mdns_label(), "dgx2");
    for not in [
        "10.10.10.2",
        "local",
        ".local",
        "dgx2.localdomain",
        "x.local.example.com",
    ] {
        assert!(!Target::parse(not).unwrap().is_mdns_name(), "{not}");
    }
}

#[test]
fn parse_all_reports_the_bad_one_and_refuses_a_duplicate() {
    let e = parse_all(&["10.10.10.2".into(), "bad:port".into()]).unwrap_err();
    assert!(e.obj.message.contains("bad:port"), "{}", e.obj.message);
    let e = parse_all(&["dgx2".into(), "dgx2:34334".into()]).unwrap_err();
    assert!(e.obj.message.contains("listed twice"), "{}", e.obj.message);
    assert_eq!(parse_all(&["a".into(), "b".into()]).unwrap().len(), 2);
}

/// A browser that hands out a fixed set of beacons and counts how often it
/// was asked.
struct FakeBrowser {
    beacons: Vec<Beacon>,
    browses: Mutex<u32>,
}

impl DiscoveryBrowser for FakeBrowser {
    fn browse(&self) -> anyhow::Result<Receiver<DiscoveryEvent>> {
        *self.browses.lock().unwrap() += 1;
        let (tx, rx) = std::sync::mpsc::channel();
        for b in &self.beacons {
            tx.send(DiscoveryEvent::Found(Box::new(b.clone()))).unwrap();
        }
        // Dropping `tx` ends the stream, so a miss returns without waiting
        // out the budget.
        Ok(rx)
    }
}

fn beacon(name: &str, ip: [u8; 4], port: u16) -> Beacon {
    Beacon {
        id: NodeId::from_bytes([7; 32]),
        name: DisplayName::new(name),
        peer_port: port,
        addresses: vec![IpAddr::V4(Ipv4Addr::from(ip))],
        can_launch: true,
        accelerator: String::new(),
    }
}

fn no_system(_: &str, _: u16) -> Vec<SocketAddr> {
    vec![]
}

#[test]
fn the_system_resolver_wins_and_the_browser_is_never_asked() {
    let b = FakeBrowser {
        beacons: vec![beacon("dgx2", [10, 10, 10, 2], 34334)],
        browses: Mutex::new(0),
    };
    let t = Target::parse("dgx2.local:4000").unwrap();
    let got = t
        .resolve_with(
            |hp, port| {
                assert_eq!(hp, "dgx2.local:4000");
                assert_eq!(port, 4000);
                vec![SocketAddr::from(([10, 10, 10, 9], port))]
            },
            Some(&b),
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(got, vec![SocketAddr::from(([10, 10, 10, 9], 4000))]);
    assert_eq!(*b.browses.lock().unwrap(), 0);
}

#[test]
fn a_local_name_the_resolver_misses_is_found_by_browsing_with_the_beacons_port() {
    let b = FakeBrowser {
        beacons: vec![
            beacon("other", [10, 10, 10, 3], 34334),
            beacon("DGX2", [10, 10, 10, 2], 34999),
        ],
        browses: Mutex::new(0),
    };
    let t = Target::parse("dgx2.local").unwrap();
    let got = t
        .resolve_with(no_system, Some(&b), Duration::from_secs(1))
        .unwrap();
    // The beacon's port, not the default the caller assumed.
    assert_eq!(got, vec![SocketAddr::from(([10, 10, 10, 2], 34999))]);
    assert_eq!(*b.browses.lock().unwrap(), 1);
}

#[test]
fn a_dns_name_never_browses_and_an_unknown_local_name_is_unreachable() {
    let b = FakeBrowser {
        beacons: vec![beacon("dgx2", [10, 10, 10, 2], 34334)],
        browses: Mutex::new(0),
    };
    // NEGATIVE CONTROL: a DNS name gets no mDNS fallback even though a
    // beacon with the matching label exists.
    let t = Target::parse("dgx2.example.com").unwrap();
    let e = t
        .resolve_with(no_system, Some(&b), Duration::from_secs(1))
        .unwrap_err();
    assert_eq!(e.obj.code, "unreachable");
    assert_eq!(*b.browses.lock().unwrap(), 0);
    // A `.local` name nobody advertises.
    let t = Target::parse("nobody.local").unwrap();
    let e = t
        .resolve_with(no_system, Some(&b), Duration::from_secs(1))
        .unwrap_err();
    assert_eq!(e.obj.code, "unreachable");
    assert!(
        e.obj.message.contains("no atlasctl agent by that name"),
        "{}",
        e.obj.message
    );
    assert_eq!(e.obj.node.as_deref(), Some("nobody.local"));
    assert_eq!(*b.browses.lock().unwrap(), 1);
    // No browser at all: same answer, no panic.
    let e = t
        .resolve_with(no_system, None, Duration::from_secs(1))
        .unwrap_err();
    assert_eq!(e.obj.code, "unreachable");
}
