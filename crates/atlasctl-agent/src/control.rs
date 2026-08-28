// SPDX-License-Identifier: AGPL-3.0-only

//! The single execution core for the seven single-node control operations.
//!
//! Extracted from the session so it has exactly two callers: the browser
//! session's local verbs, and the peer channel's terminal `Control` handler.
//! One core is the point — a second, less-checked execution path is how a
//! relayed launch skips schema validation, the `can_launch` gate, or the
//! log-line cap that the local path enforces. Everything here is I/O-free
//! except through the injected [`Launcher`] and [`LaunchTelemetry`] ports
//! (SBIO), so both callers stay testable without a container runtime.

use crate::launcher::{Launcher, Preview, Started};
use crate::logs::LogTail;
use crate::session::LaunchTelemetry;
use atlasctl_core::registry::{RecipeRef, RegistrySet};
use atlasctl_core::settings;
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::{
    AgentError, ControlRep, ControlReq, LaunchReading, RecipeInfo, RunningLaunch,
};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;

/// The seven control operations, over borrowed dependencies.
///
/// Borrowed rather than owned so the session can build one per call from its
/// existing [`SessionDeps`] without restructuring, and a daemon-side owner
/// ([`ControlHost`]) can hand the identical core to the peer channel.
///
/// [`SessionDeps`]: crate::session::SessionDeps
pub struct LocalControl<'a> {
    /// The recipe inventory.
    pub registry: &'a RegistrySet,
    /// How launches actually happen.
    pub launcher: &'a dyn Launcher,
    /// Sampling a running launch, when this agent can.
    pub telemetry: Option<&'a dyn LaunchTelemetry>,
    /// Whether this machine can run a recipe at all.
    pub can_launch: &'a Result<(), String>,
    /// What this machine's accelerator reports itself to be, for the
    /// hardware-aware refusal in [`Self::launch`]. Empty when the probe found
    /// nothing — which reads as "not GB10" and so gates nothing.
    pub accelerator: &'a str,
}

impl LocalControl<'_> {
    /// The recipe inventory, exactly as the browser's `Ready` frame lists it.
    #[must_use]
    pub fn recipes(&self) -> Vec<RecipeInfo> {
        self.registry
            .list()
            .into_iter()
            .filter_map(|entry| {
                let id = RecipeId::parse(&entry.name).ok()?;
                let r = self.registry.resolve(&RecipeRef::Bare(entry.name)).ok()?;
                let (runnable, reason) = match r.launchable() {
                    Ok(()) => (true, None),
                    Err(why) => (false, Some(why.to_string())),
                };
                Some(RecipeInfo {
                    id,
                    model: r.model.clone(),
                    nodes: r.topology.min_nodes,
                    runnable,
                    reason,
                    defaults: r
                        .defaults
                        .iter()
                        .map(|(k, v)| (k.clone(), to_wire(v)))
                        .collect(),
                })
            })
            .collect()
    }

    /// Render the command a launch would run, without running it.
    ///
    /// # Errors
    /// If the recipe is unknown, the settings are rejected, or the launcher
    /// cannot render.
    pub fn preview(
        &self,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Result<Preview, AgentError> {
        let recipe = self.resolve(recipe_id)?;
        let overrides = check_settings(requested)?;
        self.launcher.preview(&recipe, &overrides)
    }

    /// Start a recipe on this machine.
    ///
    /// # Errors
    /// If this machine cannot launch, the recipe is unknown or unlaunchable,
    /// the settings are rejected, or the launcher refuses.
    pub fn launch(
        &self,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Result<Started, AgentError> {
        if let Err(why) = self.can_launch {
            return Err(AgentError::NotLaunchable {
                recipe: recipe_id.clone(),
                reason: why.clone(),
            });
        }
        let recipe = self.resolve(recipe_id)?;
        if let Err(why) = recipe.launchable() {
            return Err(AgentError::NotLaunchable {
                recipe: recipe_id.clone(),
                reason: why.to_string(),
            });
        }
        let overrides = check_settings(requested)?;
        // A caution the CLI prints and proceeds past is worth nothing here:
        // this path is driven by the browser and by a granted remote
        // controller, and there is no operator at the keyboard to read it. On
        // a GB10 the failure it warns about is a hard freeze needing a power
        // cycle — unified memory leaves no framebuffer to fall back on — so
        // the unattended surface refuses what the attended one merely warns
        // about.
        //
        // This gates OVERRIDES only. A recipe's own value never passes through
        // here, so the shipped 0.9 profiles keep working; what is refused is a
        // number someone sent over the wire.
        for (key, value) in &overrides {
            if let atlasctl_core::ScalarValue::Float(f) = value
                && let Some(why) = atlasctl_core::settings::caution(key, *f, self.accelerator)
            {
                // `Denied` is the exact vocabulary already here: "the setting
                // exists but clients may not set it", rendered as "cannot be
                // set remotely". That is the claim being made — not that the
                // value is out of range, which it is not.
                return Err(AgentError::BadSettings {
                    errors: vec![atlasctl_protocol::settings::SettingError::Denied {
                        key: key.clone(),
                        reason: why,
                    }],
                });
            }
        }
        self.launcher.launch(&recipe, &overrides)
    }

    /// Stop a running launch, by recipe.
    ///
    /// # Errors
    /// If the launcher refuses.
    pub fn stop(&self, recipe_id: &RecipeId) -> Result<(), AgentError> {
        self.launcher.stop(recipe_id.as_str())
    }

    /// What is running on this machine.
    ///
    /// # Errors
    /// If the launcher cannot answer.
    pub fn status(&self) -> Result<Vec<RunningLaunch>, AgentError> {
        self.launcher.running()
    }

    /// How a running launch is doing.
    ///
    /// # Errors
    /// `NotReady` when this agent has no telemetry source; `NotLaunchable`
    /// when the model is not answering — the ordinary state of one still
    /// loading its weights, which the page shows as "loading", not a fault.
    pub fn stats(&self, recipe_id: &RecipeId) -> Result<LaunchReading, AgentError> {
        let Some(telemetry) = self.telemetry else {
            return Err(AgentError::NotReady);
        };
        telemetry
            .sample(recipe_id)
            .map_err(|reason| AgentError::NotLaunchable {
                recipe: recipe_id.clone(),
                reason,
            })
    }

    /// The tail of a launch's log. THIS machine caps the line count — a
    /// caller-supplied cap would let a remote requester bypass the bound.
    ///
    /// # Errors
    /// `NotReady` when this agent has no telemetry source; `NotLaunchable`
    /// when there is no container for that recipe.
    pub fn logs(&self, recipe_id: &RecipeId, lines: u32) -> Result<LogTail, AgentError> {
        let Some(telemetry) = self.telemetry else {
            return Err(AgentError::NotReady);
        };
        telemetry
            .logs(recipe_id, crate::logs::clamp_lines(lines))
            .map_err(|reason| AgentError::NotLaunchable {
                recipe: recipe_id.clone(),
                reason,
            })
    }

    /// Execute one relayed control request through the identical methods the
    /// browser session uses.
    ///
    /// Success maps verb-for-verb onto the matching [`ControlRep`] variant;
    /// a refusal is returned as the error so the caller — who knows whose
    /// refusal it is — can name itself in `ControlRep::Refused`.
    ///
    /// # Errors
    /// Whatever the underlying operation refuses with.
    pub fn execute(&self, req: ControlReq) -> Result<ControlRep, AgentError> {
        match req {
            ControlReq::ListRecipes => Ok(ControlRep::Recipes {
                recipes: self.recipes(),
            }),
            ControlReq::Preview { recipe, settings } => {
                let p = self.preview(&recipe, &settings)?;
                Ok(ControlRep::Previewed {
                    command: p.command,
                    unapplied: p.unapplied,
                })
            }
            ControlReq::Launch { recipe, settings } => {
                let started = self.launch(&recipe, &settings)?;
                Ok(ControlRep::Started {
                    recipe,
                    container: started.container,
                    endpoint: started.endpoint,
                })
            }
            ControlReq::Stop { recipe } => {
                self.stop(&recipe)?;
                Ok(ControlRep::Stopped { recipe })
            }
            ControlReq::Status => Ok(ControlRep::Status {
                running: self.status()?,
            }),
            ControlReq::Stats { recipe } => {
                let stats = self.stats(&recipe)?;
                Ok(ControlRep::Stats { recipe, stats })
            }
            ControlReq::Logs { recipe, lines } => {
                let tail = self.logs(&recipe, lines)?;
                Ok(ControlRep::Logs {
                    recipe,
                    container: tail.container,
                    lines: tail.lines,
                    running: tail.running,
                })
            }
        }
    }

    /// Resolve a recipe id against the compiled-in set.
    ///
    /// The id is already syntactically valid — it could not have been
    /// deserialized otherwise — so this only answers "does it exist here".
    fn resolve(&self, id: &RecipeId) -> Result<atlasctl_core::Recipe, AgentError> {
        self.registry
            .resolve(&RecipeRef::Bare(id.as_str().to_string()))
            .map_err(|_| AgentError::UnknownRecipe {
                recipe: id.to_string(),
            })
    }
}

/// Validate requested settings against the schema.
///
/// Denied keys are not swallowed: they ride inside the returned
/// `BadSettings` error, and the browser session extracts them for its
/// denied-attempt log — so the record survives the extraction of this core.
fn check_settings(
    requested: &BTreeMap<String, SettingValue>,
) -> Result<BTreeMap<String, atlasctl_core::ScalarValue>, AgentError> {
    settings::validate(requested).map_err(|errors| AgentError::BadSettings { errors })
}

/// A recipe default, as the wire carries it.
fn to_wire(v: &atlasctl_core::ScalarValue) -> SettingValue {
    use atlasctl_core::ScalarValue as S;
    match v {
        S::Bool(b) => SettingValue::Bool(*b),
        S::Int(i) => SettingValue::Int(*i),
        S::Float(f) => SettingValue::Float(*f),
        S::Str(s) => SettingValue::Str(s.clone()),
    }
}

/// Owns what a [`LocalControl`] borrows, for the caller that has no session.
///
/// The browser session borrows its dependencies from [`AgentState`]; the
/// daemon's peer listener outlives any session and needs its own. This
/// bundles them so the peer channel cannot be wired with half the deps —
/// every field is required at construction (PCND).
///
/// [`AgentState`]: crate::server::AgentState
pub struct ControlHost {
    registry: RegistrySet,
    launcher: Box<dyn Launcher>,
    telemetry: Option<Box<dyn LaunchTelemetry>>,
    can_launch: Result<(), String>,
    accelerator: String,
}

impl ControlHost {
    /// Bundle the dependencies of the control core.
    #[must_use]
    pub fn new(
        registry: RegistrySet,
        launcher: Box<dyn Launcher>,
        telemetry: Option<Box<dyn LaunchTelemetry>>,
        can_launch: Result<(), String>,
        accelerator: String,
    ) -> Self {
        Self {
            registry,
            launcher,
            telemetry,
            can_launch,
            accelerator,
        }
    }

    /// The control core over this host's dependencies.
    #[must_use]
    pub fn control(&self) -> LocalControl<'_> {
        LocalControl {
            accelerator: &self.accelerator,
            registry: &self.registry,
            launcher: self.launcher.as_ref(),
            telemetry: self.telemetry.as_deref(),
            can_launch: &self.can_launch,
        }
    }
}

impl std::fmt::Debug for ControlHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlHost").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "control/tests.rs"]
mod tests;
