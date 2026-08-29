// SPDX-License-Identifier: AGPL-3.0-only

//! Two-phase pairing at the session boundary.
//!
//! Split from `session/tests.rs` on the 500-line cap. The seam is real: these
//! are about when trust is written, and the rest of that file is about launches
//! and the handshake.

use super::tests::{Fixture, TOKEN};
use atlasctl_protocol::{ClientMsg, ServerMsg};

// ---- two-phase pairing ---------------------------------------------------
//
// The states worth pinning are the refusals. A confirm that works is the easy
// half; a confirm that must NOT work is where an unverified pin would sneak in.

/// Confirming with nothing pending must refuse rather than guess.
///
/// The words belong to one exchange. A confirm arriving without one — a
/// reconnected tab, a replayed frame, a second click after the socket dropped —
/// has no exchange to be about, and treating it as consent would trust a
/// machine on the strength of a message that names no key.
#[tokio::test]
async fn confirming_with_no_exchange_in_flight_trusts_nothing() {
    let f = Fixture::new();
    let mut s = f.ready().await;
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([3u8; 32]);
    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 7,
            node,
            allow_control: false,
        })
        .await;
    match &out[0] {
        ServerMsg::PairDecision {
            trusted, detail, ..
        } => {
            assert!(!trusted, "nothing may be trusted: {out:?}");
            assert!(
                detail.contains("no exchange"),
                "must say why rather than failing silently: {detail}"
            );
        }
        other => panic!("expected a decision, got {other:?}"),
    }
}

/// Rejecting with nothing pending is also honest about it, and still trusts
/// nothing.
#[tokio::test]
async fn rejecting_with_no_exchange_in_flight_is_not_an_error() {
    let f = Fixture::new();
    let mut s = f.ready().await;
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([4u8; 32]);
    match &s.handle(ClientMsg::RejectPairing { id: 8, node }).await[0] {
        ServerMsg::PairDecision { trusted, .. } => assert!(!trusted),
        other => panic!("expected a decision, got {other:?}"),
    }
}

/// Unpair answers a trust decision, not a pairing result.
///
/// It runs no exchange, so a `PairResult` would have to claim `exchanged:
/// false` — true only in the sense that nothing happened, which is not what
/// that field is for.
#[tokio::test]
async fn unpair_answers_a_decision_rather_than_an_exchange() {
    let f = Fixture::new();
    let mut s = f.ready().await;
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([5u8; 32]);
    // No fleet on this fixture, so it reports NotReady — the point here is only
    // that nothing in the pairing path answers with the wrong shape.
    let out = s.handle(ClientMsg::UnpairPeer { id: 9, node }).await;
    assert!(
        !matches!(out[0], ServerMsg::PairResult { .. }),
        "unpair must not answer with an exchange-shaped reply: {out:?}"
    );
}

/// A version 1 client must be refused outright.
///
/// It would read `exchanged` as the old `paired` and show a machine as trusted
/// that this agent has not accepted. Refusing at the handshake is the point of
/// the bump, not an inconvenience.
#[tokio::test]
async fn a_protocol_1_client_is_refused_because_it_would_misread_exchanged() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s
        .handle(ClientMsg::Hello {
            protocol_version: 1,
            token: TOKEN.into(),
        })
        .await;
    assert!(
        !matches!(out[0], ServerMsg::Ready { .. }),
        "an older client must not complete the handshake: {out:?}"
    );
}

// ---- what the exchange actually writes, and when ---------------------------
//
// The refusals above are the guard rails. These are the claim itself: that a
// completed exchange leaves NOTHING on disk, and that the pin appears at the
// confirm and names the same key whose words the operator read.
//
// The fake stands in for the network and the pin store together, because the
// real `pair` needs a second machine and a TLS handshake. What it does not fake
// is the decision logic under test — every branch below is the real
// `Session::confirm_pairing`.

use super::fleet_fake::RecordingFleet;
use crate::fleet::{FleetView, PairOutcome};
use atlasctl_protocol::fleet::{NodeDescriptor, NodeId};

/// A handshaken session wired to `fleet`.
pub(super) async fn ready_with_fleet<'a>(
    f: &'a Fixture,
    fleet: &'a dyn FleetView,
) -> super::Session<'a> {
    let (mut s, _) = super::Session::new(super::SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: f.launcher.clone(),
        token: TOKEN,
        can_launch: f.can_launch.clone(),
        fleet: Some(fleet),
        cluster: None,
        telemetry: None,
        joining: None,
        relay: None,
    });
    s.handle(ClientMsg::Hello {
        protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
        token: TOKEN.into(),
    })
    .await;
    s
}

/// Run the exchange, returning the words the operator would be shown.
pub(super) async fn exchange(s: &mut super::Session<'_>, node: NodeId) -> String {
    let out = s
        .handle(ClientMsg::PairPeer {
            id: 1,
            node,
            code: "123456".to_owned(),
        })
        .await;
    match &out[0] {
        ServerMsg::PairResult {
            exchanged,
            verification,
            ..
        } => {
            assert!(exchanged, "the exchange should have completed: {out:?}");
            verification
                .clone()
                .expect("an exchange that completed must carry words")
        }
        other => panic!("expected a pair result, got {other:?}"),
    }
}

/// THE claim. Before protocol 2 this vector held a key at this point, and the
/// dialog asking the operator to compare words was deciding nothing.
#[tokio::test]
async fn a_completed_exchange_writes_no_pin() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;

    let words = exchange(&mut s, node).await;

    assert!(!words.is_empty(), "the operator needs something to compare");
    assert!(
        fleet.keys_pinned().is_empty(),
        "the exchange completed but nothing may be trusted yet: {:?}",
        fleet.keys_pinned()
    );
}

/// The pin must name the key whose words were shown — not merely *a* key. A
/// confirm that pinned something else would make the comparison theatre: the
/// operator would have verified one machine and trusted another.
#[tokio::test]
async fn confirming_pins_exactly_the_key_whose_words_were_shown() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 2,
            node,
            allow_control: false,
        })
        .await;

    match &out[0] {
        ServerMsg::PairDecision { trusted, .. } => assert!(trusted, "{out:?}"),
        other => panic!("expected a decision, got {other:?}"),
    }
    assert_eq!(fleet.keys_pinned(), vec![fleet.outcome.public_key.clone()]);
}

/// A replayed or double-clicked confirm must not pin twice, and must not leave
/// the words live for a third.
#[tokio::test]
async fn a_second_confirm_pins_nothing_further() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    s.handle(ClientMsg::ConfirmPairing {
        id: 2,
        node,
        allow_control: false,
    })
    .await;
    let again = s
        .handle(ClientMsg::ConfirmPairing {
            id: 3,
            node,
            allow_control: false,
        })
        .await;

    match &again[0] {
        ServerMsg::PairDecision { trusted, .. } => {
            assert!(!trusted, "the exchange was already spent: {again:?}");
        }
        other => panic!("expected a decision, got {other:?}"),
    }
    assert_eq!(
        fleet.keys_pinned().len(),
        1,
        "pinned more than once: {:?}",
        fleet.keys_pinned()
    );
}

/// Confirming a *different* machine than the one that was verified must pin
/// nothing — and must also spend the exchange, so the mismatch cannot be
/// retried into a success.
#[tokio::test]
async fn confirming_a_node_the_exchange_was_not_about_pins_nothing() {
    let f = Fixture::new();
    let paired = NodeId::from_bytes([6; 32]);
    let other = NodeId::from_bytes([9; 32]);
    let fleet = RecordingFleet::new(paired);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, paired).await;

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 2,
            node: other,
            allow_control: false,
        })
        .await;
    match &out[0] {
        ServerMsg::PairDecision { trusted, .. } => assert!(!trusted, "{out:?}"),
        other => panic!("expected a decision, got {other:?}"),
    }

    // Spent, not merely refused.
    let retry = s
        .handle(ClientMsg::ConfirmPairing {
            id: 3,
            node: paired,
            allow_control: false,
        })
        .await;
    match &retry[0] {
        ServerMsg::PairDecision { trusted, .. } => assert!(!trusted, "{retry:?}"),
        other => panic!("expected a decision, got {other:?}"),
    }
    assert!(fleet.keys_pinned().is_empty(), "{:?}", fleet.keys_pinned());
}

/// Rejecting is the whole point of the change: it must leave no pin, and it
/// must consume the exchange so a later confirm cannot resurrect it.
#[tokio::test]
async fn rejecting_pins_nothing_and_spends_the_exchange() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    s.handle(ClientMsg::RejectPairing { id: 2, node }).await;
    let after = s
        .handle(ClientMsg::ConfirmPairing {
            id: 3,
            node,
            allow_control: false,
        })
        .await;

    match &after[0] {
        ServerMsg::PairDecision { trusted, .. } => {
            assert!(!trusted, "a rejected exchange must not be confirmable");
        }
        other => panic!("expected a decision, got {other:?}"),
    }
    assert!(fleet.keys_pinned().is_empty(), "{:?}", fleet.keys_pinned());
}

/// When the pin cannot be written the answer must be "not trusted". Reporting
/// success here would tell the operator a machine is paired when this side has
/// no pin for it — and the peer, which did complete the exchange, may well
/// trust them back. That asymmetry is invisible unless this says so.
#[tokio::test]
async fn a_pin_that_cannot_be_written_is_not_reported_as_trust() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let mut fleet = RecordingFleet::new(node);
    fleet.fail_pin = true;
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 2,
            node,
            allow_control: false,
        })
        .await;

    match &out[0] {
        ServerMsg::PairDecision {
            trusted, detail, ..
        } => {
            assert!(!trusted, "{out:?}");
            assert!(
                detail.contains("disk full"),
                "the operator cannot act on a failure they cannot see: {detail}"
            );
        }
        other => panic!("expected a decision, got {other:?}"),
    }
    assert!(fleet.keys_pinned().is_empty());
}

/// Words go stale. An exchange left open for hours — a tab forgotten on a
/// screen — must not still be confirmable, because nobody can now say what was
/// on the other machine when those words were produced.
#[tokio::test]
async fn an_exchange_older_than_the_ttl_cannot_be_confirmed() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    // Age it past the window rather than waiting ten minutes. Reaching into the
    // field keeps the test honest about WHICH clock the production code reads:
    // if `confirm_pairing` ever stops consulting `at`, this fails.
    let pending = s
        .pending_pairing
        .as_mut()
        .expect("the exchange should be pending");
    // Bringing the deadline forward, rather than pushing the start back: an
    // `Instant` a full TTL in the past does not exist on a machine that booted
    // more recently than that, which is every CI runner.
    pending.expires_at = std::time::Instant::now();

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 2,
            node,
            allow_control: false,
        })
        .await;

    match &out[0] {
        ServerMsg::PairDecision {
            trusted, detail, ..
        } => {
            assert!(!trusted, "{out:?}");
            assert!(detail.contains("too old"), "unhelpful refusal: {detail}");
        }
        other => panic!("expected a decision, got {other:?}"),
    }
    assert!(fleet.keys_pinned().is_empty());
}

// ---- what a join invitation offers to dial ---------------------------------

/// A fleet whose only local node is on Wi-Fi, plus loopback.
struct WirelessFleet(NodeDescriptor);

impl WirelessFleet {
    fn new() -> Self {
        use atlasctl_protocol::fleet::{
            DisplayName, Launchability, LinkClass, NodeAddress, PairingState,
        };
        let addr = |iface: &str, a: &str, class| NodeAddress {
            iface: iface.to_owned(),
            addr: a.to_owned(),
            prefix_len: 24,
            class,
            speed_mbps: None,
            rdma: false,
        };
        Self(NodeDescriptor {
            id: NodeId::from_bytes([1; 32]),
            name: DisplayName::new("laptop"),
            is_local: true,
            pairing: PairingState::Paired,
            addresses: vec![
                addr("lo", "127.0.0.1", LinkClass::Loopback),
                addr("wlp3s0", "192.168.1.24", LinkClass::Wireless),
            ],
            launchability: Launchability::yes(),
            agent_version: "0.1.3".to_owned(),
            accelerator: String::new(),
            os: "Linux".to_owned(),
            vitals: None,
            alerts: Vec::new(),
            running: None,
            vouched_by: None,
            reached_via: None,
        })
    }
}

impl FleetView for WirelessFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        vec![self.0.clone()]
    }
    fn pair(&self, _node: NodeId, _code: &str) -> anyhow::Result<PairOutcome> {
        anyhow::bail!("not used")
    }

    fn pair_at(&self, _target: &str, _code: &str) -> anyhow::Result<PairOutcome> {
        anyhow::bail!("not used")
    }
    fn trust(&self, _outcome: &PairOutcome, _allow_control: bool) -> anyhow::Result<()> {
        Ok(())
    }
    fn unpair(&self, _node: NodeId) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// The machine that mints an invitation is, by construction, one that cannot
/// run models — usually a laptop, usually on Wi-Fi. Offering it no address left
/// the page building an empty command and drawing an empty box with a Copy
/// button beside it, which is what an operator reported.
#[tokio::test]
async fn an_invitation_from_a_wireless_machine_still_carries_an_address() {
    let f = Fixture::new();
    let fleet = WirelessFleet::new();
    let window = crate::joining::JoinWindow::default();
    let (mut s, _) = super::Session::new(super::SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: f.launcher.clone(),
        token: TOKEN,
        can_launch: f.can_launch.clone(),
        fleet: Some(&fleet),
        cluster: None,
        telemetry: None,
        joining: Some(&window),
        relay: None,
    });
    s.handle(ClientMsg::Hello {
        protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
        token: TOKEN.into(),
    })
    .await;

    let out = s
        .handle(ClientMsg::MintJoinCode {
            id: 1,
            allow_control: false,
        })
        .await;

    match &out[0] {
        ServerMsg::JoinInvitation {
            code, addresses, ..
        } => {
            assert!(code.is_some(), "a window should have opened: {out:?}");
            assert_eq!(
                addresses,
                &vec!["192.168.1.24".to_owned()],
                "the Wi-Fi address must be offered and loopback must not"
            );
        }
        other => panic!("expected an invitation, got {other:?}"),
    }
}
