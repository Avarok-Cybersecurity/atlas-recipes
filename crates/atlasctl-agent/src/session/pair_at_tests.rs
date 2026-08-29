// SPDX-License-Identifier: AGPL-3.0-only

//! Pairing with an address the operator typed.
//!
//! Split from `session/pairing_tests.rs` on the 500-line cap. The seam is real:
//! that file is about when trust is written, this one is about reaching a
//! machine discovery never saw.
//!
//! mDNS is link-local. It does not cross a router and it is switched off on
//! plenty of managed networks, so "pair with something you can see" is not a
//! complete answer for anyone whose machines are not on one broadcast domain.

use super::fleet_fake::RecordingFleet;
use super::tests::{Fixture, TOKEN};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::{ClientMsg, ServerMsg};

async fn session<'a>(f: &'a Fixture, fleet: &'a RecordingFleet) -> super::Session<'a> {
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

/// The reply must name who answered. The operator typed an address, not an
/// identity, so this is the first moment anyone can say which machine it was —
/// and they have to be told before they are asked to trust it.
#[tokio::test]
async fn pairing_at_an_address_reports_the_identity_that_answered() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([7; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = session(&f, &fleet).await;

    let out = s
        .handle(ClientMsg::PairPeerAt {
            id: 1,
            target: "10.10.10.2".to_owned(),
            code: "12345678".to_owned(),
        })
        .await;

    match &out[0] {
        ServerMsg::PairAtResult {
            node: got,
            name,
            exchanged,
            verification,
            ..
        } => {
            assert_eq!(*got, Some(node));
            assert_eq!(name, "spark-b");
            assert!(exchanged);
            assert!(verification.is_some());
        }
        other => panic!("expected a pair-at result, got {other:?}"),
    }

    // Still two-phase: the exchange is held, not written.
    assert!(
        fleet.keys_pinned().is_empty(),
        "typing an address must not skip the word comparison: {:?}",
        fleet.keys_pinned()
    );
}

/// The exchange is confirmable exactly as a discovered one is — the confirm
/// path must not care how the machine was reached.
#[tokio::test]
async fn an_address_paired_exchange_is_confirmed_the_same_way() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([7; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = session(&f, &fleet).await;
    s.handle(ClientMsg::PairPeerAt {
        id: 1,
        target: "10.10.10.2:34334".to_owned(),
        code: "12345678".to_owned(),
    })
    .await;

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
    assert_eq!(fleet.keys_pinned().len(), 1);
}

/// Nothing answered, so there is no identity — and the reply must say so rather
/// than name one. A zero id here would be a claim about a machine that was
/// never reached.
#[tokio::test]
async fn an_address_that_answers_nothing_names_no_node() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([7; 32]);
    let fleet = RecordingFleet::new(node);
    fleet.start_failing();
    let mut s = session(&f, &fleet).await;

    let out = s
        .handle(ClientMsg::PairPeerAt {
            id: 1,
            target: "192.0.2.1".to_owned(),
            code: "12345678".to_owned(),
        })
        .await;

    match &out[0] {
        ServerMsg::PairAtResult {
            node,
            exchanged,
            verification,
            detail,
            ..
        } => {
            assert_eq!(*node, None, "nothing answered, so no identity may be named");
            assert!(!exchanged);
            assert!(verification.is_none());
            assert!(!detail.is_empty(), "the operator needs to know why");
        }
        other => panic!("expected a pair-at result, got {other:?}"),
    }
}

/// A failed attempt must not leave an earlier exchange confirmable. The dialog
/// says "that did not work" while, before this, a stale exchange sat behind it
/// with live words.
#[tokio::test]
async fn a_failed_attempt_spends_any_exchange_already_pending() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([7; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = session(&f, &fleet).await;
    s.handle(ClientMsg::PairPeerAt {
        id: 1,
        target: "10.10.10.2".to_owned(),
        code: "12345678".to_owned(),
    })
    .await;

    // A second attempt that fails.
    fleet.start_failing();
    s.handle(ClientMsg::PairPeer {
        id: 2,
        node: NodeId::from_bytes([8; 32]),
        code: "12345678".to_owned(),
    })
    .await;

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 3,
            node,
            allow_control: false,
        })
        .await;
    match &out[0] {
        ServerMsg::PairDecision { trusted, .. } => {
            assert!(!trusted, "the earlier exchange must not survive a failure");
        }
        other => panic!("expected a decision, got {other:?}"),
    }
    assert!(fleet.keys_pinned().is_empty());
}

/// A failure reply must carry the CAUSE, not just the attempt.
///
/// `anyhow`'s `to_string()` renders only the outermost context, so a reply built
/// from it says "resolving not-a-host" and drops "Name or service not known" —
/// the half that tells the operator whether to fix a typo, a DNS server, or a
/// firewall. `{e:#}` renders the chain.
#[tokio::test]
async fn a_failure_detail_carries_the_cause_and_not_only_the_attempt() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([7; 32]);
    let fleet = RecordingFleet::new(node);
    fleet.start_failing();
    let mut s = session(&f, &fleet).await;

    let out = s
        .handle(ClientMsg::PairPeerAt {
            id: 1,
            target: "not-a-host".to_owned(),
            code: "12345678".to_owned(),
        })
        .await;

    match &out[0] {
        ServerMsg::PairAtResult { detail, .. } => {
            assert!(
                detail.contains("resolving not-a-host"),
                "the attempt: {detail}"
            );
            assert!(
                detail.contains("Name or service not known"),
                "the cause is the half an operator can act on: {detail}"
            );
        }
        other => panic!("expected a pair-at result, got {other:?}"),
    }
}
