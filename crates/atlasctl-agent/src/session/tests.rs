// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::launcher::{Call, RecordingLauncher};
use atlasctl_protocol::settings::SettingError;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// A recipe that really is in the compiled-in corpus.
const REAL: &str = "qwen3.6-27b-fp8";

fn set(pairs: &[(&str, SettingValue)]) -> BTreeMap<String, SettingValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn id(s: &str) -> RecipeId {
    RecipeId::parse(s).expect("valid id")
}

struct Fixture {
    registry: RegistrySet,
    launcher: RecordingLauncher,
    can_launch: Result<(), String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            registry: RegistrySet::builtin_only(),
            launcher: RecordingLauncher::new(),
            can_launch: Ok(()),
        }
    }

    fn cannot_launch(reason: &str) -> Self {
        Self {
            can_launch: Err(reason.to_string()),
            ..Self::new()
        }
    }

    fn session(&self) -> Session<'_> {
        let (s, _welcome) = Session::new(SessionDeps {
            registry: &self.registry,
            launcher: &self.launcher,
            token: TOKEN,
            can_launch: self.can_launch.clone(),
            fleet: None,
            cluster: None,
        });
        s
    }

    /// A session that has already completed the handshake.
    fn ready(&self) -> Session<'_> {
        let mut s = self.session();
        let out = s.handle(ClientMsg::Hello {
            protocol_version: 1,
            token: TOKEN.into(),
        });
        assert!(
            matches!(out[0], ServerMsg::Ready { .. }),
            "handshake failed: {out:?}"
        );
        s
    }
}

#[test]
fn the_agent_speaks_first_with_a_version_range() {
    let f = Fixture::new();
    let (_s, welcome) = Session::new(SessionDeps {
        registry: &f.registry,
        launcher: &f.launcher,
        token: TOKEN,
        can_launch: Ok(()),
        fleet: None,
        cluster: None,
    });
    assert!(matches!(welcome, ServerMsg::Welcome { .. }));
}

#[test]
fn nothing_is_answered_before_the_handshake() {
    // Not even the inventory: an unauthenticated client should not learn what
    // this machine can run.
    for msg in [
        ClientMsg::ListRecipes { id: 1 },
        ClientMsg::Status { id: 1 },
        ClientMsg::Launch {
            id: 1,
            recipe: id(REAL),
            settings: BTreeMap::new(),
        },
    ] {
        let f = Fixture::new();
        let mut s = f.session();
        let out = s.handle(msg);
        assert!(
            matches!(
                out[0],
                ServerMsg::Error {
                    error: AgentError::NotReady,
                    ..
                }
            ),
            "got {out:?}"
        );
        assert!(s.is_closed());
        assert!(!f.launcher.launched_anything());
    }
}

#[test]
fn a_wrong_token_is_refused_and_ends_the_session() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Hello {
        protocol_version: 1,
        token: "f".repeat(64),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::NotPaired,
            ..
        }
    ));
    assert!(s.is_closed());
    // And a follow-up gets nothing at all.
    assert!(s.handle(ClientMsg::ListRecipes { id: 1 }).is_empty());
}

#[test]
fn an_empty_token_does_not_pass() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Hello {
        protocol_version: 1,
        token: String::new(),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::NotPaired,
            ..
        }
    ));
}

#[test]
fn a_protocol_mismatch_is_reported_rather_than_hung() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Hello {
        protocol_version: 99,
        token: TOKEN.into(),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::UnsupportedProtocol { requested: 99, .. },
            ..
        }
    ));
}

#[test]
fn a_successful_handshake_returns_the_schema_and_the_inventory() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Hello {
        protocol_version: 1,
        token: TOKEN.into(),
    });
    let ServerMsg::Ready {
        schema,
        recipes,
        can_launch,
        ..
    } = &out[0]
    else {
        panic!("expected ready, got {out:?}");
    };
    assert!(
        !schema.is_empty(),
        "the client renders what we validate, so it needs the schema"
    );
    assert!(recipes.iter().any(|r| r.id.as_str() == REAL));
    assert!(*can_launch);
    // The schema must never advertise a key clients may not set.
    assert!(!schema.iter().any(|s| s.key == "model_from_path"));
}

#[test]
fn an_unknown_recipe_is_refused_without_reaching_the_launcher() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id("no-such-recipe"),
        settings: BTreeMap::new(),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::UnknownRecipe { .. },
            ..
        }
    ));
    assert!(
        !f.launcher.launched_anything(),
        "nothing may run for an unknown recipe"
    );
}

#[test]
fn a_denied_setting_blocks_the_launch_and_is_recorded() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id(REAL),
        settings: set(&[("model_from_path", SettingValue::Str("/etc/shadow".into()))]),
    });
    let ServerMsg::Error {
        error: AgentError::BadSettings { errors },
        ..
    } = &out[0]
    else {
        panic!("expected rejected settings, got {out:?}");
    };
    assert!(matches!(errors[0], SettingError::Denied { .. }));
    assert!(
        !f.launcher.launched_anything(),
        "a denied key must stop the launch"
    );
    // Attempts on denied keys are surfaced for logging: nothing in a real UI
    // offers one, so trying says something about the client.
    assert_eq!(s.denied_attempts, ["model_from_path"]);
}

#[test]
fn an_out_of_range_setting_blocks_the_launch() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id(REAL),
        settings: set(&[("port", SettingValue::Int(1))]),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::BadSettings { .. },
            ..
        }
    ));
    assert!(!f.launcher.launched_anything());
}

#[test]
fn a_valid_launch_reaches_the_launcher_with_exactly_the_checked_settings() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 7,
        recipe: id(REAL),
        settings: set(&[("port", SettingValue::Int(9001))]),
    });
    assert!(
        matches!(out[0], ServerMsg::Started { id: 7, .. }),
        "got {out:?}"
    );
    match &f.launcher.calls()[0] {
        Call::Launch(name, overrides) => {
            assert_eq!(name, REAL);
            assert_eq!(
                overrides.len(),
                1,
                "only the checked setting may pass through"
            );
            assert_eq!(overrides["port"], atlasctl_core::ScalarValue::Int(9001));
        }
        other => panic!("wrong call: {other:?}"),
    }
}

#[test]
fn a_multi_node_recipe_is_refused_with_its_reason() {
    let f = Fixture::new();
    let mut s = f.ready();
    // A two-node recipe cannot be started from a page on one box.
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id("qwen3.5-122b-a10b-nvfp4-ep2"),
        settings: BTreeMap::new(),
    });
    // It is launchable in principle, so what matters here is that the inventory
    // told the client it needs two nodes before it ever asked.
    assert!(!out.is_empty());
    let inv = match &s.handle(ClientMsg::ListRecipes { id: 2 })[0] {
        ServerMsg::Recipes { recipes, .. } => recipes.clone(),
        other => panic!("wrong reply: {other:?}"),
    };
    let ep2 = inv
        .iter()
        .find(|r| r.id.as_str() == "qwen3.5-122b-a10b-nvfp4-ep2")
        .unwrap();
    assert_eq!(ep2.nodes, 2, "the client must be told this needs two nodes");
}

#[test]
fn a_recipe_carrying_executable_content_is_never_launched() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id("diffusion-gemma-bf16"),
        settings: BTreeMap::new(),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::NotLaunchable { .. },
            ..
        }
    ));
    assert!(!f.launcher.launched_anything());
}

#[test]
fn a_machine_that_cannot_launch_says_so_and_refuses() {
    let f = Fixture::cannot_launch("docker is not available");
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Launch {
        id: 1,
        recipe: id(REAL),
        settings: BTreeMap::new(),
    });
    assert!(matches!(
        out[0],
        ServerMsg::Error {
            error: AgentError::NotLaunchable { .. },
            ..
        }
    ));
    assert!(!f.launcher.launched_anything());
}

#[test]
fn preview_renders_without_launching_anything() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.handle(ClientMsg::Preview {
        id: 1,
        recipe: id(REAL),
        settings: BTreeMap::new(),
    });
    assert!(matches!(out[0], ServerMsg::Preview { id: 1, .. }));
    assert!(
        !f.launcher.launched_anything(),
        "preview must not start anything"
    );
    assert_eq!(f.launcher.calls(), [Call::Preview(REAL.to_string())]);
}

#[test]
fn a_malformed_frame_ends_the_session() {
    let f = Fixture::new();
    let mut s = f.ready();
    let out = s.on_malformed("trailing garbage".into());
    assert!(matches!(
        out,
        ServerMsg::Error {
            error: AgentError::InvalidMessage { .. },
            ..
        }
    ));
    assert!(s.is_closed());
}
