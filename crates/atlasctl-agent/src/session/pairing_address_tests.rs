// SPDX-License-Identifier: AGPL-3.0-only

//! What a pairing invitation carries on a machine with no wired address.
//!
//! Split from `pairing_tests.rs` for the 500-line cap, along the seam that was
//! already there: everything here is about the ADDRESS in an invitation, not
//! about the confirm/reject decisions next door.

use super::tests::{Fixture, TOKEN};
use crate::fleet::{FleetView, PairOutcome};
use atlasctl_protocol::fleet::{NodeDescriptor, NodeId};
use atlasctl_protocol::{ClientMsg, ServerMsg};

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
    fn pair<'a>(
        &'a self,
        _node: NodeId,
        _code: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<PairOutcome>> + Send + 'a>>
    {
        Box::pin(async move { anyhow::bail!("not used") })
    }

    fn pair_at<'a>(
        &'a self,
        _target: &'a str,
        _code: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<PairOutcome>> + Send + 'a>>
    {
        Box::pin(async move { anyhow::bail!("not used") })
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
