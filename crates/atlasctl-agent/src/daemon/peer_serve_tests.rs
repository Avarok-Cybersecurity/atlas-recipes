// SPDX-License-Identifier: AGPL-3.0-only

//! Refusal tests for the control-serving rules, over in-memory streams.
//!
//! Everything here runs against the REAL dispatch (`serve_peer_connection`)
//! with a real pin store on disk — only the network and the container
//! runtime are faked. The rules that need an actual second and third agent
//! (R4's dial, R5, R6's behavioural half) are proven in `relay_tests.rs`;
//! this file proves every refusal that must happen BEFORE any dial.

use super::peer_serve::{PeerServe, serve_peer_connection};
use crate::control::ControlHost;
use crate::identity::{Identity, PinStore};
use crate::launcher::{Launcher, RecordingLauncher};
use crate::peer::wire::{
    PEER_PROTOCOL_MAX, PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame,
};
use atlasctl_core::registry::RegistrySet;
use atlasctl_protocol::fleet::{DisplayName, Launchability, NodeId};
use atlasctl_protocol::msg::{AgentError, ControlRep, ControlReq};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-peerserve-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&p).expect("scratch");
        Self(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A shared handle onto the recording launcher, so a test can inspect what
/// the serving path did after handing ownership to the [`ControlHost`].
#[derive(Clone)]
struct SharedLauncher(Arc<RecordingLauncher>);

impl Launcher for SharedLauncher {
    fn preview(
        &self,
        recipe: &atlasctl_core::Recipe,
        o: &std::collections::BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Preview, AgentError> {
        self.0.preview(recipe, o)
    }
    fn launch(
        &self,
        recipe: &atlasctl_core::Recipe,
        o: &std::collections::BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Started, AgentError> {
        self.0.launch(recipe, o)
    }
    fn stop(&self, recipe: &str) -> Result<(), AgentError> {
        self.0.stop(recipe)
    }
    fn running(&self) -> Result<Vec<atlasctl_protocol::msg::RunningLaunch>, AgentError> {
        self.0.running()
    }
}

/// A rank service that must never be reached by a control frame.
struct Unreachable;

impl crate::rank::RankService for Unreachable {
    fn render(&self, _: &crate::cluster::RankAssignment) -> anyhow::Result<(String, Vec<String>)> {
        panic!("a control frame must not reach the rank service");
    }
    fn content_hash(&self, _: &str) -> anyhow::Result<String> {
        panic!("a control frame must not reach the rank service");
    }
    fn recipe_port(&self, _: &str) -> anyhow::Result<Option<u16>> {
        panic!("a control frame must not reach the rank service");
    }
    fn prepare(&self, _: &str, _: &crate::cluster::RankAssignment) -> crate::cluster::PrepareReply {
        panic!("a control frame must not reach the rank service");
    }
    fn commit(&self, _: &str) -> anyhow::Result<String> {
        panic!("a control frame must not reach the rank service");
    }
    fn alive(&self, _: &str) -> anyhow::Result<bool> {
        panic!("a control frame must not reach the rank service");
    }
    fn stop(&self, _: &str) -> anyhow::Result<()> {
        panic!("a control frame must not reach the rank service");
    }
    fn abort(&self, _: &str) {
        panic!("a control frame must not reach the rank service");
    }
}

struct Rig {
    _tmp: Tmp,
    pins: PinStore,
    launcher: Arc<RecordingLauncher>,
    serve: Arc<PeerServe>,
    local: NodeId,
}

fn rig(tag: &str) -> Rig {
    let tmp = Tmp::new(tag);
    let identity = Arc::new(Identity::load_or_create(&tmp.0).expect("identity"));
    let local = identity.id();
    let pins = PinStore::new(&tmp.0);
    let fleet = Arc::new(crate::fleet::LocalFleet::new(
        Identity::load_or_create(&tmp.0).expect("same identity from disk"),
        pins.clone(),
        DisplayName::new("serving-box"),
        Vec::new(),
        Launchability::yes(),
        "GB10".to_owned(),
    ));
    let launcher = Arc::new(RecordingLauncher::new());
    let serve = Arc::new(PeerServe {
        identity,
        pins: pins.clone(),
        fleet,
        rank: Arc::new(Unreachable),
        control: Arc::new(ControlHost::new(
            RegistrySet::builtin_only(),
            Box::new(SharedLauncher(Arc::clone(&launcher))),
            None,
            Ok(()),
        )),
        peer_port: 34334,
        // Tests never reach a real dial; the budget only has to exist.
        answer_budget: Duration::from_secs(2),
    });
    Rig {
        _tmp: tmp,
        pins,
        launcher,
        serve,
        local,
    }
}

fn sender() -> NodeId {
    NodeId::from_bytes([0x5e; 32])
}

fn pin_sender(pins: &PinStore, granted: bool) {
    crate::fleet::record_pairing(pins, sender(), "aa", DisplayName::new("requester"), 0, None)
        .expect("pin");
    if granted {
        assert!(pins.set_controller(sender(), true).expect("grant"));
    }
}

/// Drive one serving task and return a client-side stream that has already
/// completed the hello exchange.
async fn connected(rig: &Rig) -> (tokio::io::DuplexStream, tokio::task::JoinHandle<()>) {
    let (mut ours, mut theirs) = tokio::io::duplex(1 << 16);
    let serve = Arc::clone(&rig.serve);
    let s = sender();
    let task = tokio::spawn(async move {
        serve_peer_connection(&mut theirs, &serve, s).await;
    });
    write_frame(
        &mut ours,
        &PeerFrame::Hello {
            version: PEER_PROTOCOL_VERSION,
            name: "client".to_owned(),
            can_launch: false,
            accelerator: String::new(),
            os: "Linux".to_owned(),
            addresses: Vec::new(),
            version_max: Some(PEER_PROTOCOL_MAX),
            vouched: None,
        },
    )
    .await
    .expect("hello");
    let hello = read_frame(&mut ours).await.expect("hello back");
    assert!(matches!(hello, PeerFrame::Hello { .. }));
    (ours, task)
}

async fn ask(stream: &mut tokio::io::DuplexStream, frame: &PeerFrame) -> ControlRep {
    write_frame(stream, frame).await.expect("send");
    match read_frame(stream).await.expect("answered, not dropped") {
        PeerFrame::ControlReply { rep } => rep,
        other => panic!("expected a control reply, got {other:?}"),
    }
}

/// T2 — the retroactive-widening regression test: a peer that is PINNED but
/// not granted `controller` sends a terminal `Control` directly and is
/// refused, with the exact grant command in the refusal. This is the test
/// that fails if anyone reintroduces "pinned = authorized".
#[tokio::test]
async fn a_pinned_but_ungranted_sender_is_refused_terminal_control() {
    let r = rig("t2");
    pin_sender(&r.pins, false);
    let (mut c, task) = connected(&r).await;

    let rep = ask(
        &mut c,
        &PeerFrame::Control {
            req: ControlReq::Status,
        },
    )
    .await;
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::ControlRefused { reason, .. },
        } => {
            assert_eq!(by, r.local, "the refusal must name who refused");
            assert!(
                reason.contains("peer grant-control"),
                "the fix must be copy-paste: {reason}"
            );
        }
        other => panic!("expected ControlRefused, got {other:?}"),
    }
    assert!(
        r.launcher.calls().is_empty(),
        "an ungranted sender must not reach the launcher: {:?}",
        r.launcher.calls()
    );
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// T2/T3 — with the grant, the verb executes through the shared core.
#[tokio::test]
async fn a_granted_sender_executes_through_the_local_core() {
    let r = rig("granted");
    pin_sender(&r.pins, true);
    let (mut c, task) = connected(&r).await;

    let rep = ask(
        &mut c,
        &PeerFrame::Control {
            req: ControlReq::Status,
        },
    )
    .await;
    assert!(matches!(rep, ControlRep::Status { .. }), "got {rep:?}");
    assert!(
        r.launcher
            .calls()
            .iter()
            .any(|c| matches!(c, crate::launcher::Call::Running)),
        "the status must have come from the launcher"
    );
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// T2 — revocation is immediate: the pin file is re-read per frame, so a
/// grant withdrawn between two requests refuses the second WITHOUT any
/// restart or reconnect.
#[tokio::test]
async fn revoking_the_grant_takes_effect_on_the_very_next_frame() {
    let r = rig("revoke");
    pin_sender(&r.pins, true);
    let (mut c, task) = connected(&r).await;

    let first = ask(
        &mut c,
        &PeerFrame::Control {
            req: ControlReq::Status,
        },
    )
    .await;
    assert!(matches!(first, ControlRep::Status { .. }));

    assert!(r.pins.set_controller(sender(), false).expect("revoke"));

    let second = ask(
        &mut c,
        &PeerFrame::Control {
            req: ControlReq::Status,
        },
    )
    .await;
    assert!(
        matches!(
            second,
            ControlRep::Refused {
                error: AgentError::ControlRefused { .. },
                ..
            }
        ),
        "a revocation that needs a restart is not a revocation: {second:?}"
    );
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// T3 — a relayed launch runs the same schema validation the browser path
/// runs; garbage settings are refused before the launcher is touched.
#[tokio::test]
async fn a_relayed_launch_is_schema_validated_before_the_launcher() {
    let r = rig("schema");
    pin_sender(&r.pins, true);
    let (mut c, task) = connected(&r).await;

    let rep = ask(
        &mut c,
        &PeerFrame::Control {
            req: ControlReq::Launch {
                recipe: atlasctl_protocol::RecipeId::parse("qwen3.6-27b-fp8").expect("id"),
                settings: [(
                    "no_such_setting".to_owned(),
                    atlasctl_protocol::settings::SettingValue::Bool(true),
                )]
                .into_iter()
                .collect(),
            },
        },
    )
    .await;
    assert!(
        matches!(
            rep,
            ControlRep::Refused {
                error: AgentError::BadSettings { .. },
                ..
            }
        ),
        "got {rep:?}"
    );
    assert!(!r.launcher.launched_anything());
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// R2 — the relay checks the requester's OWN grant before doing anything on
/// its behalf; an ungranted requester cannot spend the relay's authority at
/// a third machine (the confused deputy).
#[tokio::test]
async fn an_ungranted_sender_cannot_ask_for_a_forward() {
    let r = rig("r2");
    pin_sender(&r.pins, false);
    let (mut c, task) = connected(&r).await;

    let target = NodeId::from_bytes([0x7a; 32]);
    let rep = ask(
        &mut c,
        &PeerFrame::ControlTo {
            node: target,
            req: ControlReq::Status,
        },
    )
    .await;
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { node, detail },
        } => {
            assert_eq!(by, r.local);
            assert_eq!(node, target, "the refusal names the TARGET");
            assert!(
                detail.contains("peer grant-control"),
                "the fix must be copy-paste: {detail}"
            );
        }
        other => panic!("expected RelayRefused, got {other:?}"),
    }
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// R3 — a relay forwards only to machines in its OWN pin store: a vouched or
/// invented target is refused, which is the one-hop knowledge rule enforced
/// where it can be enforced (a relay cannot relay a relay).
#[tokio::test]
async fn a_relay_refuses_to_reach_a_node_it_has_not_itself_pinned() {
    let r = rig("r3");
    pin_sender(&r.pins, true);
    let (mut c, task) = connected(&r).await;

    let stranger = NodeId::from_bytes([0x7a; 32]);
    let rep = ask(
        &mut c,
        &PeerFrame::ControlTo {
            node: stranger,
            req: ControlReq::Status,
        },
    )
    .await;
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { node, detail },
        } => {
            assert_eq!(by, r.local);
            assert_eq!(node, stranger);
            assert!(detail.contains("not a peer"), "got {detail}");
        }
        other => panic!("expected RelayRefused, got {other:?}"),
    }
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// R4, the fail-closed half provable without a network: a pinned target with
/// no address in the relay's OWN state is refused — nothing else (a frame, a
/// digest) can supply one, because nothing else is consulted.
#[tokio::test]
async fn a_pinned_target_with_no_known_address_is_refused_not_guessed() {
    let r = rig("r4");
    pin_sender(&r.pins, true);
    let target = NodeId::from_bytes([0x7a; 32]);
    crate::fleet::record_pairing(&r.pins, target, "bb", DisplayName::new("t"), 0, None)
        .expect("pin");
    let (mut c, task) = connected(&r).await;

    let rep = ask(
        &mut c,
        &PeerFrame::ControlTo {
            node: target,
            req: ControlReq::Status,
        },
    )
    .await;
    match rep {
        ControlRep::Refused {
            error: AgentError::RelayRefused { detail, .. },
            ..
        } => assert!(detail.contains("no known address"), "got {detail}"),
        other => panic!("expected RelayRefused, got {other:?}"),
    }
    drop(c);
    task.await.expect("serving task ends cleanly");
}

/// A pairing frame arriving mid-serving is refused by dispatch exactly as it
/// always was: the conversation ends, and no control machinery sees it.
#[tokio::test]
async fn a_pairing_frame_mid_control_serving_ends_the_conversation() {
    let r = rig("pairmid");
    pin_sender(&r.pins, true);
    let (mut c, task) = connected(&r).await;

    write_frame(
        &mut c,
        &PeerFrame::PairStart {
            message: "aa".to_owned(),
        },
    )
    .await
    .expect("send");
    // The server hangs up rather than answering; the read fails.
    assert!(read_frame(&mut c).await.is_err());
    drop(c);
    task.await.expect("serving task ends cleanly");
}
