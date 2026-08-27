// SPDX-License-Identifier: AGPL-3.0-only

//! The protocol-4 forwarding surface, in the build that carries it ahead of
//! the router.
//!
//! The `on` annotation and `allow_control` exist on the wire before the code
//! that honours them (the session router and the `Pin::controller` store are
//! later steps of the forwarding design). These tests pin the placeholder
//! behaviour that makes the intermediate build safe: a request aimed at
//! another machine is REFUSED, never executed here as if it were local, and
//! consent that cannot be recorded is refused rather than silently dropped.
//! Each would fail loudly if someone made the placeholders "helpful".

use super::fleet_fake::RecordingFleet;
use super::pairing_tests::{exchange, ready_with_fleet};
use super::tests::Fixture;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::AgentError;
use atlasctl_protocol::{ClientMsg, ServerMsg};
use std::collections::BTreeMap;

fn target() -> NodeId {
    NodeId::from_bytes([0xd6; 32])
}

fn recipe() -> atlasctl_protocol::RecipeId {
    atlasctl_protocol::RecipeId::parse("qwen3.6-27b-fp8").expect("valid fixture id")
}

/// Every one of the seven forwardable verbs, aimed at another node.
fn forwarded(on: NodeId) -> Vec<ClientMsg> {
    let on = Some(on);
    vec![
        ClientMsg::ListRecipes { id: 1, on },
        ClientMsg::Preview {
            id: 2,
            recipe: recipe(),
            settings: BTreeMap::new(),
            on,
        },
        ClientMsg::Launch {
            id: 3,
            recipe: recipe(),
            settings: BTreeMap::new(),
            on,
        },
        ClientMsg::Stop {
            id: 4,
            recipe: recipe(),
            on,
        },
        ClientMsg::Status { id: 5, on },
        ClientMsg::LaunchStats {
            id: 6,
            recipe: recipe(),
            on,
        },
        ClientMsg::LaunchLogs {
            id: 7,
            recipe: recipe(),
            lines: 50,
            on,
        },
    ]
}

#[test]
fn a_verb_aimed_at_another_node_is_refused_not_executed_here() {
    // The dangerous failure mode is a Launch{on: dgx2} starting a model on
    // THIS box while the page reports it running on dgx2. Every forwardable
    // verb must come back as a typed refusal naming the node, with the
    // launcher untouched.
    let f = Fixture::new();
    let mut s = f.ready();
    for msg in forwarded(target()) {
        let sent_id = match &msg {
            ClientMsg::ListRecipes { id, .. }
            | ClientMsg::Preview { id, .. }
            | ClientMsg::Launch { id, .. }
            | ClientMsg::Stop { id, .. }
            | ClientMsg::Status { id, .. }
            | ClientMsg::LaunchStats { id, .. }
            | ClientMsg::LaunchLogs { id, .. } => *id,
            other => panic!("not a forwardable verb: {other:?}"),
        };
        let out = s.handle(msg);
        match &out[0] {
            ServerMsg::Error {
                id: Some(id),
                error: AgentError::NotRoutable { node, .. },
            } => {
                assert_eq!(*id, sent_id, "refusal must answer the request it refuses");
                assert_eq!(
                    *node,
                    target(),
                    "refusal must name the node it cannot reach"
                );
            }
            other => panic!("expected NotRoutable, got {other:?}"),
        }
    }
    assert!(
        !f.launcher.launched_anything(),
        "a forwarded verb must never reach this machine's launcher"
    );
    assert!(
        f.launcher.calls().is_empty(),
        "not even a read: {:?}",
        f.launcher.calls()
    );
    assert!(
        !s.is_closed(),
        "a routing refusal is an answer, not a fault"
    );
}

#[test]
fn an_unauthenticated_socket_cannot_probe_the_routing_surface() {
    // The refusal happens after the handshake gate, so an unauthenticated
    // client gets the same NotReady-and-close as for any other verb — it
    // must not learn whether this agent can route anywhere.
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Status {
        id: 1,
        on: Some(target()),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::NotReady,
            ..
        }
    ));
    assert!(s.is_closed());
}

#[test]
fn minting_with_allow_control_is_refused_not_silently_dropped() {
    // The grant store does not exist in this build. Minting the code anyway
    // would hand the operator a pairing they believe carries the controller
    // grant when nothing recorded it — so the request is refused, typed.
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::MintJoinCode {
        id: 9,
        allow_control: true,
    });
    assert!(
        matches!(
            &out[0],
            ServerMsg::Error {
                id: Some(9),
                error: AgentError::InvalidMessage { .. },
            }
        ),
        "expected a typed refusal, got {out:?}"
    );
    assert!(!s.is_closed(), "a refusal is an answer, not a fault");
}

/// The grant half of protocol 4, in the build that carries the field ahead of
/// the store. `Pin::controller` is a later step of the forwarding design, so
/// a confirm asking for the grant is refused whole: writing the pin while
/// dropping the grant would leave the operator believing consent to remote
/// control was recorded when nothing was.
#[test]
fn confirming_with_allow_control_is_refused_and_pins_nothing() {
    let f = Fixture::new();
    let node = NodeId::from_bytes([6; 32]);
    let fleet = RecordingFleet::new(node);
    let mut s = ready_with_fleet(&f, &fleet);
    exchange(&mut s, node);

    let out = s.handle(ClientMsg::ConfirmPairing {
        id: 2,
        node,
        allow_control: true,
    });
    assert!(
        matches!(
            &out[0],
            ServerMsg::Error {
                id: Some(2),
                error: AgentError::InvalidMessage { .. },
            }
        ),
        "expected a typed refusal, got {out:?}"
    );
    assert!(
        fleet.keys_pinned().is_empty(),
        "a refused confirm must trust nothing: {:?}",
        fleet.keys_pinned()
    );
}
