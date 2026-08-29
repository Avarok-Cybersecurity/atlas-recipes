// SPDX-License-Identifier: AGPL-3.0-only

//! The three-agent suite: origin → relay → target over real localhost TLS,
//! driving the production `ControlDriver` against the production serving
//! path. The happy path first, then the lying-intermediary cases and the
//! forward-loop bound. The grant matrix, version skew and budgets live in
//! `relay_grant_tests.rs`.

use super::relay_harness::*;
use crate::session::ControlRelay as _;
use atlasctl_protocol::fleet::{NodeAddress, NodeId, PairingState};
use atlasctl_protocol::msg::{AgentError, ControlRep, ControlReq};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// The whole design, end to end: the target is known to the origin ONLY
/// through the relay's digest, and a launch aimed at it rides the relay —
/// while the digest writes no pin, the listing says exactly what is known
/// second-hand, and a fourth machine the target pins is never dialled.
#[tokio::test(flavor = "multi_thread")]
async fn a_vouched_launch_rides_the_relay_end_to_end() {
    let origin = agent("hp-origin", "macbook", "127.0.0.1");
    let mut relay = agent("hp-relay", "dgx1", "127.0.0.2");
    let mut target = agent("hp-target", "dgx2", "127.0.0.3");
    let fourth = agent("hp-fourth", "dgx3", "127.0.0.4");

    // Trust, hop by hop: the origin and relay know each other, the relay and
    // target know each other. The target has never heard of the origin.
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    pin(&relay, &target, false);
    pin(&target, &relay, true);
    // The residual-bound case: the target itself pins a fourth node.
    pin(&target, &fourth, false);

    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(10), launcher)
    }
    .await;
    {
        let launcher = std::sync::Arc::clone(&target.launcher);
        spawn_serving(&mut target, port, Duration::from_secs(10), launcher)
    }
    .await;
    let fourth_dials = counting_listener("127.0.0.4", port).await;

    // The relay hears from the target first-hand; the origin then polls the
    // relay and learns of the target ONLY from the digest.
    poll(&relay, &target).await;
    let pins_before = origin.pin_file_bytes();
    let digest = poll(&origin, &relay).await.expect("a v2 peer sends one");
    assert!(
        digest.iter().any(|v| v.node == target.id() && v.reachable),
        "the digest must vouch for the reachable target: {digest:?}"
    );
    assert_eq!(
        origin.pin_file_bytes(),
        pins_before,
        "ingesting a digest must never write a pin"
    );

    // The listing renders the claim as a claim.
    use crate::fleet::FleetView as _;
    let listed = origin
        .fleet
        .nodes()
        .into_iter()
        .find(|n| n.id == target.id())
        .expect("the vouched target is part of the fleet view");
    assert_eq!(listed.pairing, PairingState::Vouched);
    assert_eq!(listed.vouched_by, Some(relay.id()));
    assert_eq!(listed.reached_via, Some(relay.id()));

    // Drive the launch. The origin holds no pin and no address for the
    // target; everything it can do goes through the relay.
    let d = driver(&origin, port);
    let (rep, via) = d
        .control(
            target.id(),
            ControlReq::Launch {
                recipe: recipe(),
                settings: BTreeMap::new(),
            },
        )
        .await
        .expect("routable");
    assert!(
        matches!(rep, ControlRep::Started { .. }),
        "the launch must reach the target: {rep:?}"
    );
    assert_eq!(
        via,
        Some(relay.id()),
        "provenance: forwarded means via = relay"
    );
    assert!(
        target.launcher.launched_anything(),
        "the TARGET ran the launch"
    );
    assert!(
        !relay.launcher.launched_anything(),
        "the relay must execute nothing itself"
    );
    assert_eq!(
        fourth_dials.load(Ordering::SeqCst),
        0,
        "a terminal Control cannot make the target dial anyone (T4)"
    );

    // Stop scoping: the relayed Stop names the recipe and stops exactly the
    // launcher-managed launch; nothing in `ControlReq` can name a container.
    let (rep, _) = d
        .control(target.id(), ControlReq::Stop { recipe: recipe() })
        .await
        .expect("routable");
    assert!(matches!(rep, ControlRep::Stopped { .. }));
    assert!(
        target
            .launcher
            .calls()
            .iter()
            .any(|c| matches!(c, crate::launcher::Call::Stop(r) if r == recipe().as_str())),
        "the stop resolved a recipe, never a container"
    );
}

/// Lying intermediary — identity. A fabricated node in the digest is listed
/// only as a claim, writes no pin, and dies at the honest relay's R3.
#[tokio::test(flavor = "multi_thread")]
async fn a_fabricated_vouch_writes_no_pin_and_is_refused_at_the_relay() {
    let origin = agent("fab-origin", "macbook", "127.0.0.1");
    let mut relay = agent("fab-relay", "dgx1", "127.0.0.2");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    poll(&origin, &relay).await;

    // The relay's digest — as a compromised relay would send it — vouches
    // for a machine that does not exist.
    let phantom = NodeId::from_bytes([0xfa; 32]);
    let pins_before = origin.pin_file_bytes();
    origin
        .fleet
        .record_vouches(relay.id(), vec![claim(phantom, Vec::new(), true)]);
    assert_eq!(
        origin.pin_file_bytes(),
        pins_before,
        "a fabricated vouch must never reach the pin store"
    );

    use crate::fleet::FleetView as _;
    let listed = origin
        .fleet
        .nodes()
        .into_iter()
        .find(|n| n.id == phantom)
        .expect("listed, as a claim");
    assert_eq!(listed.pairing, PairingState::Vouched);
    assert_eq!(listed.vouched_by, Some(relay.id()));

    // Control toward it goes to the (honest) relay, which does not pin it.
    let (rep, via) = driver(&origin, port)
        .control(phantom, status())
        .await
        .expect("the route exists; the relay answers");
    assert_eq!(via, Some(relay.id()));
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { node, via, detail },
        } => {
            assert_eq!(by, relay.id(), "the honest relay refuses under R3");
            assert_eq!(node, phantom);
            assert_eq!(
                via,
                Some(relay.id()),
                "the error itself must name the relay: `by` is dropped when \
                 this becomes a browser frame, so attribution that lives only \
                 there sends the operator to the target instead"
            );
            assert!(detail.contains("not a peer"), "got {detail}");
        }
        other => panic!("expected the relay's R3 refusal, got {other:?}"),
    }
}

/// Lying intermediary — address. A hostile address in the digest entry for a
/// REAL node is never dialled by anyone: the origin routes through the
/// relay, and the relay resolves from its own reports (R4).
#[tokio::test(flavor = "multi_thread")]
async fn a_hostile_claimed_address_is_never_dialled_by_anyone() {
    let origin = agent("addr-origin", "macbook", "127.0.0.1");
    let mut relay = agent("addr-relay", "dgx1", "127.0.0.2");
    let mut target = agent("addr-target", "dgx2", "127.0.0.3");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    pin(&relay, &target, false);
    pin(&target, &relay, true);

    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    {
        let launcher = std::sync::Arc::clone(&target.launcher);
        spawn_serving(&mut target, port, Duration::from_secs(5), launcher)
    }
    .await;
    let hostile = counting_listener("127.0.0.7", port).await;

    poll(&relay, &target).await;
    poll(&origin, &relay).await;
    // Overwrite the honest claim with one carrying the hostile address —
    // the strongest position a lying relay can reach.
    origin.fleet.record_vouches(
        relay.id(),
        vec![claim(
            target.id(),
            vec![NodeAddress {
                iface: String::new(),
                addr: "127.0.0.7".to_owned(),
                class: atlasctl_protocol::fleet::LinkClass::Roce,
                speed_mbps: Some(200_000),
                prefix_len: 8,
                rdma: true,
            }],
            true,
        )],
    );

    let (rep, via) = driver(&origin, port)
        .control(target.id(), status())
        .await
        .expect("routable");
    assert!(matches!(rep, ControlRep::Status { .. }), "got {rep:?}");
    assert_eq!(via, Some(relay.id()));
    assert_eq!(
        hostile.load(Ordering::SeqCst),
        0,
        "a claimed address is display data; it must receive zero connections"
    );
}

/// Lying intermediary — liveness. The relay claims a node reachable that it
/// cannot actually dial: the answer is the RELAY's refusal, within budget,
/// never dressed as the target's.
#[tokio::test(flavor = "multi_thread")]
async fn a_liveness_lie_comes_back_as_the_relays_own_refusal() {
    let origin = agent("live-origin", "macbook", "127.0.0.1");
    let mut relay = agent("live-relay", "dgx1", "127.0.0.2");
    let ghost = agent("live-ghost", "dgx9", "127.0.0.9");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    // The relay pins the ghost (so R3 passes) but nothing listens there.
    pin(&relay, &ghost, false);

    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    poll(&origin, &relay).await;
    origin
        .fleet
        .record_vouches(relay.id(), vec![claim(ghost.id(), Vec::new(), true)]);

    let started = std::time::Instant::now();
    let (rep, via) = driver(&origin, port)
        .control(ghost.id(), status())
        .await
        .expect("the relay answers, even when its leg fails");
    assert!(
        started.elapsed() < crate::peer::control::ORIGIN_ANSWER_BUDGET,
        "the refusal must arrive inside the origin budget"
    );
    assert_eq!(via, Some(relay.id()));
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { node, .. },
        } => {
            assert_eq!(by, relay.id(), "the RELAY owns this failure");
            assert_eq!(node, ghost.id());
        }
        other => panic!(
            "a dial failure must surface as the relay's RelayRefused, never \
             as the target's refusal: {other:?}"
        ),
    }
}

/// Forward loop (R6, behavioural half): the frame the target receives for a
/// forwarded request is the TERMINAL `Control` — the relay writes no
/// `ControlTo` onward, so a chain longer than one hop cannot exist on the
/// wire.
#[tokio::test(flavor = "multi_thread")]
async fn the_relay_emits_only_a_terminal_frame_at_the_target() {
    let origin = agent("loop-origin", "macbook", "127.0.0.1");
    let mut relay = agent("loop-relay", "dgx1", "127.0.0.2");
    let target = agent("loop-target", "dgx2", "127.0.0.3");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    pin(&relay, &target, false);
    pin(&target, &relay, true);

    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    // The instrumented target records every frame it is sent.
    let (_, frames) = spawn_fake_peer(&target, port, Some(2), Duration::ZERO).await;

    poll(&origin, &relay).await;
    origin
        .fleet
        .record_vouches(relay.id(), vec![claim(target.id(), Vec::new(), true)]);
    // R4 needs the relay's own address for the target; a fake peer answers
    // no poll digests, so hand the relay a live report the way its poll
    // loop would.
    relay.fleet.record_report(crate::peer::link::PeerReport {
        node: target.id(),
        name: "dgx2".to_owned(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link: atlasctl_protocol::fleet::LinkClass::Ethernet,
        addresses: vec![NodeAddress {
            iface: "lo".to_owned(),
            addr: "127.0.0.3".to_owned(),
            class: atlasctl_protocol::fleet::LinkClass::Ethernet,
            speed_mbps: None,
            prefix_len: 8,
            rdma: false,
        }],
        vouched: None,
        peer_version_max: 2,
    });

    let (rep, _) = driver(&origin, port)
        .control(target.id(), status())
        .await
        .expect("routable");
    assert!(matches!(rep, ControlRep::Status { .. }), "got {rep:?}");

    let seen = frames.lock().expect("lock").clone();
    assert!(
        !seen.is_empty(),
        "the target must have been reached through the relay"
    );
    for f in &seen {
        assert!(
            matches!(f, crate::peer::wire::PeerFrame::Control { .. }),
            "the ONLY control frame a relay may emit is the terminal one: {f:?}"
        );
    }
}

/// The origin cannot reach the RELAY at all — a distinct failure from the
/// relay's own leg failing (covered above), and the one the browser most
/// often shows, because a laptop that has moved networks still holds the
/// relay's stale address.
///
/// Both refusals say `RelayRefused` and both name the TARGET in `node`, so
/// the only thing separating "go look at dgx1" from "go look at dgx2" is
/// `via`. This path builds the error on the ORIGIN, where `ControlRep` never
/// exists and `by` therefore cannot carry the blame.
#[tokio::test(flavor = "multi_thread")]
async fn a_relay_the_origin_cannot_dial_is_named_in_the_refusal() {
    let origin = agent("dead-origin", "macbook", "127.0.0.1");
    let mut relay = agent("dead-relay", "dgx1", "127.0.0.2");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    poll(&origin, &relay).await;

    let ghost = agent("dead-ghost", "dgx2", "127.0.0.3");
    origin
        .fleet
        .record_vouches(relay.id(), vec![claim(ghost.id(), Vec::new(), true)]);

    // A port nothing is listening on: bound to learn a free number, then
    // dropped. The route still resolves through the relay — it is only the
    // dial that fails, which is exactly the operator's situation.
    let dead = {
        let l = tokio::net::TcpListener::bind("127.0.0.2:0")
            .await
            .expect("bind to learn a free port");
        l.local_addr().expect("port").port()
    };
    assert_ne!(dead, port, "the dead port must not be the relay's live one");

    let err = driver(&origin, dead)
        .control(ghost.id(), status())
        .await
        .expect_err("nothing is listening; the dial cannot succeed");
    match err {
        AgentError::RelayRefused { node, via, detail } => {
            assert_eq!(node, ghost.id(), "`node` stays the target");
            assert_eq!(
                via,
                Some(relay.id()),
                "the unreachable RELAY must be named, or the operator is sent \
                 to a target that never heard of the request"
            );
            assert!(
                detail.contains("could not ask"),
                "the prose must say which leg failed: {detail}"
            );
        }
        other => panic!("expected the origin-side RelayRefused, got {other:?}"),
    }
}
