// SPDX-License-Identifier: AGPL-3.0-only

//! Consent recorded at the pairing ceremony.
//!
//! Split from `session/forward_tests.rs` on the 500-line cap. The seam is real:
//! that file is about where a verb is EXECUTED, and this one is about whether a
//! peer was ever granted the right to send one at all.

use super::fleet_fake::RecordingFleet;
use super::pairing_tests::{exchange, ready_with_fleet};
use super::tests::{Fixture, TOKEN};
use super::{Session, SessionDeps};

/// The id this fake agent answers to.
fn local_id() -> NodeId {
    NodeId::from_bytes([1; 32])
}
use atlasctl_protocol::fleet::{Launchability, NodeDescriptor, NodeId, PairingState};
use atlasctl_protocol::{ClientMsg, ServerMsg};

/// A fleet that knows only which node is this machine — the one fact O1
/// needs.
struct SelfAwareFleet;

impl crate::fleet::FleetView for SelfAwareFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        vec![NodeDescriptor {
            id: local_id(),
            name: atlasctl_protocol::fleet::DisplayName::new("this-box"),
            is_local: true,
            pairing: PairingState::Paired,
            addresses: Vec::new(),
            launchability: Launchability::yes(),
            agent_version: String::new(),
            accelerator: String::new(),
            os: String::new(),
            vitals: None,
            alerts: Vec::new(),
            running: None,
            vouched_by: None,
            reached_via: None,
        }]
    }
    fn pair<'a>(
        &'a self,
        _: NodeId,
        _: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<crate::fleet::PairOutcome>> + Send + 'a,
        >,
    > {
        Box::pin(async move { anyhow::bail!("not under test") })
    }
    fn pair_at<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = anyhow::Result<crate::fleet::PairOutcome>> + Send + 'a,
        >,
    > {
        Box::pin(async move { anyhow::bail!("not under test") })
    }
    fn trust(&self, _: &crate::fleet::PairOutcome, _: bool) -> anyhow::Result<()> {
        anyhow::bail!("not under test")
    }
    fn unpair(&self, _: NodeId) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// Confirming with `allow_control` writes the grant with the pin — one
/// human decision, recorded atomically.
#[tokio::test]
async fn confirming_with_allow_control_records_the_grant_with_the_trust() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet).await;
    exchange(&mut s, node).await;

    let out = s
        .handle(ClientMsg::ConfirmPairing {
            id: 2,
            node,
            allow_control: true,
        })
        .await;
    assert!(
        matches!(
            &out[0],
            ServerMsg::PairDecision {
                id: 2,
                trusted: true,
                ..
            }
        ),
        "got {out:?}"
    );
    assert_eq!(fleet.keys_pinned().len(), 1);
    assert_eq!(fleet.grants(), vec![node], "the grant rode the confirm");
}

/// And without it, nothing is granted — consent must be said, not implied.
#[tokio::test]
async fn confirming_without_allow_control_grants_nothing() {
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
    assert!(matches!(
        &out[0],
        ServerMsg::PairDecision { trusted: true, .. }
    ));
    assert_eq!(fleet.keys_pinned().len(), 1);
    assert!(fleet.grants().is_empty());
}

/// Minting with `allow_control` stamps the window, so the machine that
/// joins through it is pinned WITH the grant the human chose.
#[tokio::test]
async fn minting_with_allow_control_stamps_the_join_window() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let joining = crate::joining::JoinWindow::default();
    let (mut s, _) = Session::new(SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: f.launcher.clone(),
        token: TOKEN,
        can_launch: f.can_launch.clone(),
        fleet: Some(&fleet),
        cluster: None,
        telemetry: None,
        joining: Some(&joining),
        relay: None,
    });
    let out = s
        .handle(ClientMsg::Hello {
            protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
            token: TOKEN.into(),
        })
        .await;
    assert!(matches!(out[0], ServerMsg::Ready { .. }));

    let out = s
        .handle(ClientMsg::MintJoinCode {
            id: 9,
            allow_control: true,
        })
        .await;
    assert!(
        matches!(&out[0], ServerMsg::JoinInvitation { code: Some(_), .. }),
        "got {out:?}"
    );
    assert_eq!(
        joining.consume(),
        Some(crate::joining::Consumed {
            allow_control: true
        }),
        "the window must carry the grant to the pin write"
    );
}
