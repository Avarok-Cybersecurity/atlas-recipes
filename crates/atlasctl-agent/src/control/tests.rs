// SPDX-License-Identifier: AGPL-3.0-only

//! Direct tests for the shared control core.
//!
//! The session suite already proves the browser path did not change when this
//! core was extracted; these prove the properties the PEER caller relies on,
//! since it has no session of its own to inherit them from.

use super::*;
use crate::launcher::{Call, RecordingLauncher};
use atlasctl_protocol::msg::ControlReq;
use std::sync::Mutex;

/// A recipe that really is in the compiled-in corpus.
const REAL: &str = "qwen3.6-27b-fp8";

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

    fn control_on(&self, accelerator: &'static str) -> LocalControl<'_> {
        LocalControl {
            accelerator,
            registry: &self.registry,
            launcher: &self.launcher,
            telemetry: None,
            can_launch: &self.can_launch,
        }
    }

    fn control(&self) -> LocalControl<'_> {
        LocalControl {
            accelerator: "",
            registry: &self.registry,
            launcher: &self.launcher,
            telemetry: None,
            can_launch: &self.can_launch,
        }
    }
}

/// Records the line cap it was handed, so a test can prove the TARGET
/// enforced it rather than trusting whatever count arrived on the wire.
struct CapRecorder {
    asked: Mutex<Vec<u32>>,
}

impl LaunchTelemetry for CapRecorder {
    fn sample(&self, _: &RecipeId) -> Result<LaunchReading, String> {
        Err("not under test".into())
    }

    fn logs(&self, _: &RecipeId, lines: u32) -> Result<LogTail, String> {
        self.asked.lock().expect("lock").push(lines);
        Ok(LogTail {
            container: "c".into(),
            lines: Vec::new(),
            running: true,
        })
    }
}

#[test]
fn every_verb_reaches_the_launcher_through_the_same_core() {
    let f = Fixture::new();
    let c = f.control();

    assert!(matches!(
        c.execute(ControlReq::ListRecipes),
        Ok(ControlRep::Recipes { .. })
    ));
    assert!(matches!(
        c.execute(ControlReq::Preview {
            recipe: id(REAL),
            settings: BTreeMap::new(),
        }),
        Ok(ControlRep::Previewed { .. })
    ));
    assert!(matches!(
        c.execute(ControlReq::Launch {
            recipe: id(REAL),
            settings: BTreeMap::new(),
        }),
        Ok(ControlRep::Started { .. })
    ));
    assert!(matches!(
        c.execute(ControlReq::Stop { recipe: id(REAL) }),
        Ok(ControlRep::Stopped { .. })
    ));
    assert!(matches!(
        c.execute(ControlReq::Status),
        Ok(ControlRep::Status { .. })
    ));
    // The launcher saw the same calls the browser path would have made.
    let calls = f.launcher.calls();
    assert!(calls.iter().any(|c| matches!(c, Call::Preview(_))));
    assert!(calls.iter().any(|c| matches!(c, Call::Launch(..))));
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::Stop(r) if r == REAL))
    );
    assert!(calls.iter().any(|c| matches!(c, Call::Running)));
}

#[test]
fn a_relayed_launch_cannot_skip_the_can_launch_gate() {
    // The exact validation-skip the shared core exists to prevent: the local
    // path refuses on `can_launch`, so the relayed path must too.
    let f = Fixture {
        can_launch: Err("no docker here".into()),
        ..Fixture::new()
    };
    let out = f.control().execute(ControlReq::Launch {
        recipe: id(REAL),
        settings: BTreeMap::new(),
    });
    assert!(
        matches!(out, Err(AgentError::NotLaunchable { .. })),
        "got {out:?}"
    );
    assert!(!f.launcher.launched_anything());
}

#[test]
fn a_relayed_launch_cannot_skip_schema_validation() {
    let f = Fixture::new();
    let out = f.control().execute(ControlReq::Launch {
        recipe: id(REAL),
        settings: [("no_such_setting".to_owned(), SettingValue::Bool(true))]
            .into_iter()
            .collect(),
    });
    assert!(
        matches!(out, Err(AgentError::BadSettings { .. })),
        "got {out:?}"
    );
    assert!(!f.launcher.launched_anything());
}

#[test]
fn an_unknown_recipe_is_refused_by_name() {
    let f = Fixture::new();
    let out = f.control().execute(ControlReq::Preview {
        recipe: id("not-a-recipe"),
        settings: BTreeMap::new(),
    });
    assert!(
        matches!(out, Err(AgentError::UnknownRecipe { .. })),
        "got {out:?}"
    );
}

#[test]
fn the_target_caps_the_log_lines_whatever_the_wire_asked() {
    // A relay-supplied count must not bypass this machine's own bound: the
    // cap is applied HERE, after deserialization, exactly as the local
    // `LaunchLogs` path applies it.
    let f = Fixture::new();
    let telemetry = CapRecorder {
        asked: Mutex::new(Vec::new()),
    };
    let c = LocalControl {
        accelerator: "",
        registry: &f.registry,
        launcher: &f.launcher,
        telemetry: Some(&telemetry),
        can_launch: &f.can_launch,
    };
    c.execute(ControlReq::Logs {
        recipe: id(REAL),
        lines: u32::MAX,
    })
    .expect("logs");
    assert_eq!(
        telemetry.asked.lock().expect("lock").as_slice(),
        &[crate::logs::MAX_LINES],
        "the wire's u32::MAX must arrive at the source clamped"
    );
}

#[test]
fn stats_and_logs_without_a_telemetry_source_say_so() {
    let f = Fixture::new();
    let c = f.control();
    assert!(matches!(
        c.execute(ControlReq::Stats { recipe: id(REAL) }),
        Err(AgentError::NotReady)
    ));
    assert!(matches!(
        c.execute(ControlReq::Logs {
            recipe: id(REAL),
            lines: 10,
        }),
        Err(AgentError::NotReady)
    ));
}

/// A caution the CLI prints and proceeds past is worth nothing on this
/// surface: the browser and a granted remote controller drive it, and there
/// is no operator at the keyboard to read a warning. On a GB10 the failure it
/// warns about is a hard freeze needing a power cycle.
#[test]
fn a_remote_override_above_the_gb10_caution_is_refused_not_warned() {
    let f = Fixture::new();
    let c = f.control_on("NVIDIA GB10");
    let mut req = std::collections::BTreeMap::new();
    req.insert(
        "gpu_memory_utilization".to_owned(),
        atlasctl_protocol::settings::SettingValue::Float(0.95),
    );
    let err = c.launch(&id(REAL), &req).expect_err("must refuse");
    // Structured, not prose: `BadSettings`'s Display is a summary ("1
    // setting(s) rejected"), and the browser renders the errors themselves.
    let atlasctl_protocol::msg::AgentError::BadSettings { errors } = &err else {
        panic!("expected BadSettings, got {err:?}");
    };
    assert!(
        errors.iter().any(|e| matches!(
            e,
            atlasctl_protocol::settings::SettingError::Denied { key, .. }
                if key == "gpu_memory_utilization"
        )),
        "the refusal must name the setting, as Denied: {errors:?}"
    );
    assert!(
        !f.launcher.launched_anything(),
        "nothing may reach the launcher"
    );
}

/// The bound is hardware-aware, not a blanket cap: the same value on hardware
/// the caution does not know is none of its business.
#[test]
fn the_same_value_is_untouched_on_hardware_the_caution_does_not_claim() {
    let f = Fixture::new();
    let c = f.control_on("NVIDIA H100");
    let mut req = std::collections::BTreeMap::new();
    req.insert(
        "gpu_memory_utilization".to_owned(),
        atlasctl_protocol::settings::SettingValue::Float(0.95),
    );
    // Reaches the launcher — it may still fail there for unrelated reasons,
    // but it must not be refused by the GB10 caution.
    let out = c.launch(&id(REAL), &req);
    if let Err(atlasctl_protocol::msg::AgentError::BadSettings { errors }) = &out {
        assert!(
            !errors.iter().any(|e| matches!(
                e,
                atlasctl_protocol::settings::SettingError::Denied { key, .. }
                    if key == "gpu_memory_utilization"
            )),
            "refused on hardware the caution does not claim: {errors:?}"
        );
    }
}
