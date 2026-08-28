// SPDX-License-Identifier: AGPL-3.0-only

//! The session's remote-routing surface: rule O1, the relay hand-off, and
//! the provenance every reply must state.
//!
//! O2–O5 live behind the [`ControlRelay`] trait and are proven against the
//! real planner in `fleet/routing_tests.rs` and the three-agent suite in
//! `daemon/relay_tests.rs`; here a fake relay proves the SESSION's half:
//! which verbs route, what gets executed locally, and that the page is told
//! exactly what was done.

use super::tests::{Fixture, TOKEN};
use super::{ControlRelay, Session, SessionDeps};
use atlasctl_protocol::fleet::{Launchability, NodeDescriptor, NodeId, PairingState};
use atlasctl_protocol::msg::{AgentError, ControlRep, ControlReq};
use atlasctl_protocol::{ClientMsg, ServerMsg};
use std::collections::BTreeMap;
use std::sync::Mutex;

fn target() -> NodeId {
    NodeId::from_bytes([7; 32])
}

fn local_id() -> NodeId {
    NodeId::from_bytes([0x11; 32])
}

fn relay_id() -> NodeId {
    NodeId::from_bytes([0x22; 32])
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
    fn pair(&self, _: NodeId, _: &str) -> anyhow::Result<crate::fleet::PairOutcome> {
        anyhow::bail!("not under test")
    }
    fn pair_at(&self, _: &str, _: &str) -> anyhow::Result<crate::fleet::PairOutcome> {
        anyhow::bail!("not under test")
    }
    fn trust(&self, _: &crate::fleet::PairOutcome, _: bool) -> anyhow::Result<()> {
        anyhow::bail!("not under test")
    }
    fn unpair(&self, _: NodeId) -> anyhow::Result<bool> {
        Ok(false)
    }
}

/// What a scripted relay answers with.
type Scripted = Result<(ControlRep, Option<NodeId>), AgentError>;

/// Scripted relay: records what it was asked, answers from a table.
struct FakeRelay {
    calls: Mutex<Vec<(NodeId, ControlReq)>>,
    answer: Box<dyn Fn(&ControlReq) -> Scripted + Send + Sync>,
}

impl FakeRelay {
    fn new(answer: impl Fn(&ControlReq) -> Scripted + Send + Sync + 'static) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        }
    }

    fn calls(&self) -> Vec<(NodeId, ControlReq)> {
        self.calls.lock().expect("lock").clone()
    }
}

impl ControlRelay for FakeRelay {
    fn control(
        &self,
        target: NodeId,
        req: ControlReq,
    ) -> Result<(ControlRep, Option<NodeId>), AgentError> {
        self.calls.lock().expect("lock").push((target, req.clone()));
        (self.answer)(&req)
    }
}

/// A ready session over a self-aware fleet and the given relay.
fn routed_session<'a>(
    f: &'a Fixture,
    fleet: &'a SelfAwareFleet,
    relay: Option<&'a dyn ControlRelay>,
) -> Session<'a> {
    let (mut s, _) = Session::new(SessionDeps {
        accelerator: "",
        registry: &f.registry,
        launcher: &f.launcher,
        token: TOKEN,
        can_launch: f.can_launch.clone(),
        fleet: Some(fleet),
        cluster: None,
        telemetry: None,
        joining: None,
        relay,
    });
    let out = s.handle(ClientMsg::Hello {
        protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
        token: TOKEN.into(),
    });
    assert!(matches!(out[0], ServerMsg::Ready { .. }));
    s
}

/// The verb-for-verb answer a well-behaved relay would give.
fn answer_in_kind(req: &ControlReq) -> Scripted {
    let rep = match req {
        ControlReq::ListRecipes => ControlRep::Recipes {
            recipes: Vec::new(),
        },
        ControlReq::Preview { .. } => ControlRep::Previewed {
            command: "docker run …".into(),
            unapplied: Vec::new(),
        },
        ControlReq::Launch { recipe, .. } => ControlRep::Started {
            recipe: recipe.clone(),
            container: "atlas-remote".into(),
            endpoint: None,
        },
        ControlReq::Stop { recipe } => ControlRep::Stopped {
            recipe: recipe.clone(),
        },
        ControlReq::Status => ControlRep::Status {
            running: Vec::new(),
        },
        ControlReq::Stats { recipe } => ControlRep::Stats {
            recipe: recipe.clone(),
            stats: atlasctl_protocol::msg::LaunchReading::default(),
        },
        ControlReq::Logs { recipe, .. } => ControlRep::Logs {
            recipe: recipe.clone(),
            container: "atlas-remote".into(),
            lines: Vec::new(),
            running: true,
        },
    };
    Ok((rep, Some(relay_id())))
}

/// The provenance pair a reply states, or a panic for shapes without one.
fn provenance(msg: &ServerMsg) -> (Option<NodeId>, Option<NodeId>) {
    match msg {
        ServerMsg::Recipes { on, via, .. }
        | ServerMsg::Preview { on, via, .. }
        | ServerMsg::Started { on, via, .. }
        | ServerMsg::Stopped { on, via, .. }
        | ServerMsg::Status { on, via, .. }
        | ServerMsg::Stats { on, via, .. }
        | ServerMsg::Logs { on, via, .. } => (*on, *via),
        other => panic!("not a control reply: {other:?}"),
    }
}

/// No relay, no fleet: exactly the old refusal, so a single-node agent is
/// not degraded by the router existing.
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
    // The routing happens after the handshake gate, so an unauthenticated
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

/// O1 — `on: Some(local id)` is this machine: executed locally through the
/// unchanged path, relay never consulted, provenance `(None, None)`.
#[test]
fn a_verb_aimed_at_this_machine_by_id_runs_locally_and_never_dials() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let relay = FakeRelay::new(|_| panic!("O1 must never reach the relay"));
    let mut s = routed_session(&f, &fleet, Some(&relay));

    let out = s.handle(ClientMsg::Status {
        id: 5,
        on: Some(local_id()),
    });
    assert_eq!(
        provenance(&out[0]),
        (None, None),
        "local means (None, None)"
    );
    assert!(relay.calls().is_empty());
    assert!(
        f.launcher
            .calls()
            .iter()
            .any(|c| matches!(c, crate::launcher::Call::Running)),
        "the local launcher answered"
    );
}

/// Every forwardable verb maps onto its `ControlReq` twin and comes back as
/// the matching reply with honest provenance `(Some(target), via)`.
#[test]
fn each_remote_verb_maps_to_its_control_twin_with_stated_provenance() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let relay = FakeRelay::new(answer_in_kind);
    let mut s = routed_session(&f, &fleet, Some(&relay));

    for msg in forwarded(target()) {
        let out = s.handle(msg);
        assert_eq!(
            provenance(&out[0]),
            (Some(target()), Some(relay_id())),
            "forwarded means (Some(target), Some(relay)): {:?}",
            out[0]
        );
    }
    let asked: Vec<ControlReq> = relay.calls().into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        asked,
        vec![
            ControlReq::ListRecipes,
            ControlReq::Preview {
                recipe: recipe(),
                settings: BTreeMap::new(),
            },
            ControlReq::Launch {
                recipe: recipe(),
                settings: BTreeMap::new(),
            },
            ControlReq::Stop { recipe: recipe() },
            ControlReq::Status,
            ControlReq::Stats { recipe: recipe() },
            ControlReq::Logs {
                recipe: recipe(),
                lines: 50,
            },
        ]
    );
    assert!(
        f.launcher.calls().is_empty(),
        "a remote verb must not touch this machine's launcher"
    );
}

/// A direct dial reports `via: None` — the page may claim end-to-end
/// authentication only then.
#[test]
fn a_direct_answer_reports_no_relay() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let relay = FakeRelay::new(|req| answer_in_kind(req).map(|(rep, _)| (rep, None)));
    let mut s = routed_session(&f, &fleet, Some(&relay));

    let out = s.handle(ClientMsg::Status {
        id: 5,
        on: Some(target()),
    });
    assert_eq!(provenance(&out[0]), (Some(target()), None));
}

/// A refusal from the chain surfaces typed, under the browser's correlation
/// id — never dressed up as a successful reply.
#[test]
fn a_chain_refusal_surfaces_typed_under_the_request_id() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let relay = FakeRelay::new(|_| {
        Ok((
            ControlRep::Refused {
                by: relay_id(),
                error: AgentError::RelayRefused {
                    node: target(),
                    via: Some(relay_id()),
                    detail: "dial failed".into(),
                },
            },
            Some(relay_id()),
        ))
    });
    let mut s = routed_session(&f, &fleet, Some(&relay));

    let out = s.handle(ClientMsg::Stop {
        id: 4,
        recipe: recipe(),
        on: Some(target()),
    });
    assert!(
        matches!(
            &out[0],
            ServerMsg::Error {
                id: Some(4),
                error: AgentError::RelayRefused { node, via, .. },
            } if *node == target() && *via == Some(relay_id())
        ),
        "got {:?}",
        out[0]
    );
}

/// A relay answering `Stop` with someone's `Recipes` is lying or confused;
/// rendering the mismatch as the reply would misattribute it. Fail closed.
#[test]
fn a_mismatched_answer_shape_is_refused_not_rendered() {
    let f = Fixture::new();
    let fleet = SelfAwareFleet;
    let relay = FakeRelay::new(|_| {
        Ok((
            ControlRep::Recipes {
                recipes: Vec::new(),
            },
            Some(relay_id()),
        ))
    });
    let mut s = routed_session(&f, &fleet, Some(&relay));

    let out = s.handle(ClientMsg::Stop {
        id: 4,
        recipe: recipe(),
        on: Some(target()),
    });
    assert!(
        matches!(
            &out[0],
            ServerMsg::Error {
                id: Some(4),
                error: AgentError::InvalidMessage { .. },
            }
        ),
        "got {:?}",
        out[0]
    );
}
