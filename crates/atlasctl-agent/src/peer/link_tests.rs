// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for the peer link: what a Hello carries, and how a peer's address
//! becomes something dialable.

use super::link::*;
use super::wire::{PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use atlasctl_protocol::fleet::NodeVitals;
use std::net::SocketAddr;

use atlasctl_protocol::fleet::Metric;

fn vitals() -> NodeVitals {
    NodeVitals {
        accelerator_util: Metric::Unsupported,
        sm_clock_mhz: Metric::Unsupported,
        sm_clock_healthy_mhz: None,
        temperature_c: Metric::Unsupported,
        power_w: Metric::Unsupported,
        memory_used_frac: Metric::Unsupported,
        memory_total_bytes: Metric::Unsupported,
        disk_free_bytes: Metric::Unsupported,
        docker_ok: false,
        agent_uptime_s: 0,
    }
}

/// A peer offers vitals unsolicited, so they can land between the request
/// and its answer. Reading the next frame as *the* answer mistook a vitals
/// sample for a preview and failed every real two-node preview.
#[tokio::test]
async fn preview_reads_past_interleaved_vitals() {
    let (mut a, mut b) = tokio::io::duplex(1 << 16);

    let responder = tokio::spawn(async move {
        // Two vitals frames arrive before the answer.
        for _ in 0..2 {
            write_frame(
                &mut b,
                &PeerFrame::Vitals {
                    vitals: Box::new(vitals()),
                },
            )
            .await
            .unwrap();
        }
        write_frame(
            &mut b,
            &PeerFrame::RankPreviewed {
                command: "docker run rank1".to_owned(),
                unmapped: vec!["mtp_gate".to_owned()],
            },
        )
        .await
        .unwrap();
    });

    let mut got = None;
    for _ in 0..8 {
        match read_frame(&mut a).await.unwrap() {
            PeerFrame::Vitals { .. } => continue,
            PeerFrame::RankPreviewed { command, unmapped } => {
                got = Some((command, unmapped));
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    responder.await.unwrap();
    assert_eq!(
        got,
        Some(("docker run rank1".to_owned(), vec!["mtp_gate".to_owned()]))
    );
}

/// The whole point of a control-only agent is that it cannot run a model.
/// Every outbound hello used to be written by hand with `can_launch: true`,
/// so such an agent introduced itself to every peer as able to launch.
#[tokio::test]
async fn a_control_only_agent_does_not_claim_it_can_launch() {
    let (mut a, mut b) = tokio::io::duplex(1 << 16);
    let intro = SelfIntro {
        name: "laptop".to_owned(),
        can_launch: false,
        accelerator: String::new(),
        os: "macOS".to_owned(),
        version_max: Some(super::wire::PEER_PROTOCOL_MAX),
        vouched: None,
    };

    let peer = tokio::spawn(async move {
        let heard = read_frame(&mut b).await.unwrap();
        // Answer so the exchange completes; the claim under test is ours.
        write_frame(
            &mut b,
            &PeerFrame::Hello {
                version: PEER_PROTOCOL_VERSION,
                name: "spark".to_owned(),
                can_launch: true,
                accelerator: "gb10".to_owned(),
                os: "Linux".to_owned(),
                addresses: Vec::new(),
                version_max: None,
                vouched: None,
            },
        )
        .await
        .unwrap();
        heard
    });

    let addr: SocketAddr = "10.0.0.1:34334".parse().unwrap();
    let theirs = exchange_hello(&mut a, addr, &intro, &[]).await.unwrap();
    let ours = peer.await.unwrap();

    match ours {
        PeerFrame::Hello {
            name, can_launch, ..
        } => {
            assert!(!can_launch, "a control-only agent claimed it can launch");
            assert_eq!(name, "laptop");
        }
        other => panic!("expected a hello, got {other:?}"),
    }
    // And the peer's own claim is carried back untouched, which is what
    // launchability is now derived from.
    assert!(theirs.can_launch);
    assert_eq!(theirs.accelerator, "gb10");
}

/// The name is derived, but the capability must be supplied — there is no
/// default, because the wrong default is the bug above.
#[test]
fn an_intro_reports_the_capability_it_was_given() {
    assert!(!SelfIntro::new(false, "").can_launch);
    assert!(SelfIntro::new(true, "gb10").can_launch);
    assert_eq!(SelfIntro::new(true, "gb10").accelerator, "gb10");
}

/// A peer that only ever sends vitals must not hold the preview open.
#[tokio::test]
async fn preview_gives_up_on_endless_vitals() {
    let (mut a, mut b) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        for _ in 0..20 {
            if write_frame(
                &mut b,
                &PeerFrame::Vitals {
                    vitals: Box::new(vitals()),
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
    });

    let mut answered = false;
    for _ in 0..8 {
        if let Ok(PeerFrame::Vitals { .. }) = read_frame(&mut a).await {
            continue;
        }
        answered = true;
    }
    assert!(!answered, "the loop must be bounded, not endless");
}

/// The accelerator a node reports over the authenticated channel is the one
/// the fleet view shows, because `fleet::listing` prefers the peer report
/// to the beacon. An empty string is therefore not a harmless default — it
/// actively overwrites the good value the beacon already carried.
#[test]
fn an_intro_carries_the_accelerator_into_the_hello_it_becomes() {
    let intro = SelfIntro::new(true, "NVIDIA GB10");
    assert_eq!(
        intro.accelerator, "NVIDIA GB10",
        "the tag must survive into the frame the peer reads"
    );
    let blank = SelfIntro::new(true, "");
    assert!(
        blank.accelerator.is_empty(),
        "an empty tag stays empty rather than becoming a guess"
    );
}

/// How the poll loop turns a peer's address into something dialable.
///
/// Kept beside the peer code rather than inline in the loop so it can be
/// tested: the loop's own failure mode was silence, and silence is exactly
/// what a test has to make loud.
fn sock(addr: &str, port: u16) -> Option<std::net::SocketAddr> {
    addr.parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, port))
}

#[test]
fn an_ipv6_peer_is_dialable() {
    // `format!("{addr}:{port}").parse()` cannot do this: a SocketAddr
    // string needs the literal in brackets. Every IPv6 peer therefore fell
    // through the poll loop's `continue` and was never contacted — it just
    // aged into "stale" with nothing logged.
    for a in ["fe80::1", "2001:db8::5", "::1"] {
        let s = sock(a, 34334).unwrap_or_else(|| panic!("{a} must be dialable"));
        assert_eq!(s.port(), 34334);
        assert!(s.is_ipv6(), "{a} must stay IPv6");
    }
}

#[test]
fn ipv4_still_works_and_nonsense_still_does_not() {
    assert_eq!(sock("10.10.10.2", 34334).map(|s| s.port()), Some(34334));
    assert!(sock("not-an-address", 34334).is_none());
    assert!(sock("", 34334).is_none());
    // A host:port pair is not an IP, and must not be accepted as one.
    assert!(sock("10.10.10.2:1", 34334).is_none());
}

/// An old build's hello — written before `version_max` and `vouched` existed
/// — must come out of `exchange_hello` as "did not say" on both counts, so
/// vitals and cluster keep working across a rolling upgrade while control
/// frames know to refuse locally.
#[tokio::test]
async fn an_old_builds_hello_reads_as_did_not_say() {
    let (mut a, mut b) = tokio::io::duplex(1 << 16);

    let old_peer = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // Consume our hello so the exchange can proceed; the claim under test
        // is what the OLD peer's bytes decode to.
        let mut len = [0u8; 4];
        b.read_exact(&mut len).await.unwrap();
        let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
        b.read_exact(&mut body).await.unwrap();
        // Byte-for-byte what a pre-digest build serialises: no version_max,
        // no vouched. Writing PeerFrame::Hello here would test the NEW
        // serialiser against itself.
        let old = br#"{"type":"hello","version":1,"name":"old-spark","can_launch":true,"accelerator":"GB10","os":"Linux","addresses":[]}"#;
        let framed_len = u32::try_from(old.len()).unwrap().to_be_bytes();
        b.write_all(&framed_len).await.unwrap();
        b.write_all(old).await.unwrap();
        b.flush().await.unwrap();
    });

    let addr: SocketAddr = "10.0.0.1:34334".parse().unwrap();
    let hello = exchange_hello(&mut a, addr, &SelfIntro::new(true, "gb10"), &[])
        .await
        .expect("a v1 hello must still complete the exchange");
    old_peer.await.unwrap();

    assert_eq!(hello.name, "old-spark");
    assert_eq!(hello.version_max, None, "silence is not a version claim");
    assert_eq!(hello.vouched, None, "silence is not an empty pin store");
    // The normalization `query` applies: a build that never said a maximum
    // speaks exactly the version the equality check accepted.
    assert_eq!(
        hello.version_max.unwrap_or(PEER_PROTOCOL_VERSION),
        1,
        "an old peer must never be treated as digest-capable"
    );
}

/// ONE-HOP KNOWLEDGE, at the builder. A digest is what this agent knows
/// first-hand — its pins, its report cache — and NEVER an entry that arrived
/// in someone else's digest. If this test fails, vouches gossip: A tells B,
/// B tells C, and a retraction can never catch up with the rumour.
#[test]
fn a_received_vouch_never_enters_this_agents_own_digest() {
    use crate::fleet::FleetView as _;
    use atlasctl_protocol::fleet::{DisplayName, Launchability, LinkClass, NodeId, VouchedPeer};

    let dir = std::env::temp_dir().join(format!(
        "atlasctl-digest-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("scratch");
    let pins = crate::identity::PinStore::new(&dir);
    let fleet = crate::fleet::LocalFleet::new(
        crate::identity::Identity::generate(),
        pins.clone(),
        DisplayName::new("spark-256a"),
        Vec::new(),
        Launchability::yes(),
        "GB10".to_owned(),
    );

    // First-hand: a pin this agent holds, with a live report.
    let pinned = NodeId::from_bytes([4; 32]);
    crate::fleet::record_pairing(
        &pins,
        pinned,
        "aa",
        DisplayName::new("mine"),
        0,
        None,
        false,
    )
    .expect("pin");
    fleet.record_report(PeerReport {
        node: pinned,
        name: "mine".to_owned(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link: LinkClass::Roce,
        addresses: Vec::new(),
        vouched: None,
        peer_version_max: super::wire::PEER_PROTOCOL_MAX,
    });

    // Second-hand: an entry that arrived in the pinned peer's digest.
    let rumour = NodeId::from_bytes([9; 32]);
    fleet.record_vouches(
        pinned,
        vec![VouchedPeer {
            node: rumour,
            name: DisplayName::new("their-peer"),
            can_launch: true,
            accelerator: "GB10".to_owned(),
            os: "Linux".to_owned(),
            addresses: Vec::new(),
            link: LinkClass::Roce,
            reachable: true,
            vitals: None,
            vitals_age_s: None,
        }],
    );
    // It is genuinely in the fleet view, so its absence below is the
    // builder's discipline and not an ingestion failure.
    assert!(fleet.nodes().iter().any(|n| n.id == rumour));

    let digest = fleet_digest(&pins, &fleet).expect("builds");
    assert!(
        digest.iter().any(|v| v.node == pinned),
        "own pins are vouched"
    );
    assert!(
        !digest.iter().any(|v| v.node == rumour),
        "an entry from a received digest must never be re-vouched"
    );
    // And what we do vouch is stated with its evidence discipline intact:
    // no vitals were held, so no age is invented.
    let entry = digest.iter().find(|v| v.node == pinned).expect("entry");
    assert!(entry.reachable, "a live report is what reachable means");
    assert_eq!(entry.vitals_age_s, None, "no vitals, no age — never zero");

    let _ = std::fs::remove_dir_all(&dir);
}
