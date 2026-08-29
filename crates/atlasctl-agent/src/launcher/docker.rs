// SPDX-License-Identifier: AGPL-3.0-only

//! The production launcher: translate a recipe, then run the command.

use super::{Launcher, Preview, Started};
use atlasctl_core::chain::UserConfig;
use atlasctl_core::docker::collective::NcclRoce;
use atlasctl_core::docker::profile::{DeviceProfile, LaunchProfile};
use atlasctl_core::docker::translate::{
    LABEL_MANAGED, LABEL_RECIPE, LaunchContext, Placement, translate,
};
use atlasctl_core::host::HostSnapshot;
use atlasctl_core::io::ProcessRunner;
use atlasctl_core::{Recipe, ScalarValue};
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::{AgentError, RunningLaunch};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Launches recipes through the docker CLI.
pub struct DockerLauncher {
    runner: Arc<dyn ProcessRunner>,
    host: HostSnapshot,
    profile: &'static LaunchProfile,
    devices: Box<dyn DeviceProfile>,
}

impl DockerLauncher {
    /// Build a launcher.
    pub fn new(
        runner: Arc<dyn ProcessRunner>,
        host: HostSnapshot,
        profile: &'static LaunchProfile,
        devices: Box<dyn DeviceProfile>,
    ) -> Self {
        Self {
            runner,
            host,
            profile,
            devices,
        }
    }

    fn plan(
        &self,
        recipe: &Recipe,
        overrides: &BTreeMap<String, ScalarValue>,
    ) -> Result<atlasctl_core::docker::LaunchPlan, AgentError> {
        // A browser launch is always single-node. Multi-node needs a rank per
        // machine, which is a fleet decision rather than something one page
        // should be able to assert on its own.
        let ctx = LaunchContext {
            profile: self.profile,
            devices: self.devices.as_ref(),
            collective: &NcclRoce,
        };
        let mut plan = translate(
            recipe,
            overrides,
            &UserConfig::new(),
            &self.host,
            &Placement::Solo,
            &ctx,
        )
        .map_err(|e| AgentError::NotLaunchable {
            recipe: RecipeId::parse(&recipe.name)
                .unwrap_or_else(|_| RecipeId::parse("unknown").expect("literal is a valid id")),
            reason: format!("{e:#}"),
        })?;
        // The agent removes a container by name before it starts one, and on
        // stop, so it owns this lifecycle already. Auto-remove therefore buys
        // nothing and costs the only evidence there is: a rank that dies takes
        // its logs with it, and the operator's next question is always "why".
        // That happened for real -- a rank died a second after starting and the
        // container was gone before anyone could read it.
        plan.docker.auto_remove = false;
        Ok(plan)
    }
}

impl Launcher for DockerLauncher {
    fn preview(
        &self,
        recipe: &Recipe,
        overrides: &BTreeMap<String, ScalarValue>,
    ) -> Result<Preview, AgentError> {
        let plan = self.plan(recipe, overrides)?;
        Ok(Preview {
            command: plan.docker.to_string(),
            // Settings the recipe carries that this build does not understand.
            // Reported so the client can show them: the tool this replaces
            // discarded them silently.
            unapplied: plan.unmapped.into_iter().map(|u| u.key).collect(),
        })
    }

    fn launch(
        &self,
        recipe: &Recipe,
        overrides: &BTreeMap<String, ScalarValue>,
    ) -> Result<Started, AgentError> {
        let plan = self.plan(recipe, overrides)?;
        let name = plan.docker.name.clone();

        // The same guard `atlasctl run` applies, because this path has an
        // operator too — they are just in a browser instead of a terminal, and
        // they get LESS to work with, not more: the launch runs offline, so a
        // missing model fails inside a container that has already exited, and
        // all the browser can show is that it stopped.
        //
        // Skipped when the recipe brings its own weights (`--model-from-path`),
        // since then the Hub cache is never consulted.
        if !plan.docker.command.iter().any(|a| a == "--model-from-path")
            && let Some(dir) = atlasctl_core::hfcache::hub_dir(
                std::path::Path::new(&self.host.hf_cache_dir),
                &recipe.model,
            )
        {
            use atlasctl_core::hfcache::CacheState;
            let detail = match atlasctl_core::hfcache::cache_state(&dir) {
                CacheState::Weights => None,
                CacheState::Absent => Some(format!(
                    "the model `{}` is not in this machine's HuggingFace cache ({}). \
                     The launch runs offline, so it cannot download it — fetch it \
                     first with `hf download {}`.",
                    recipe.model, self.host.hf_cache_dir, recipe.model
                )),
                // Named separately: telling someone it is "not there" when they
                // can see the directory reads as a broken tool, and they stop
                // trusting the instruction that follows.
                CacheState::MetadataOnly => Some(format!(
                    "the model `{}` has a cache directory in {} but no weight \
                     files — what an interrupted or metadata-only download \
                     leaves behind. Complete it with `hf download {}`.",
                    recipe.model, self.host.hf_cache_dir, recipe.model
                )),
            };
            if let Some(detail) = detail {
                return Err(AgentError::LaunchFailed { detail });
            }
        }

        // Clear a previous container of the same name. Failure is expected when
        // nothing is there; the launch below is what has to succeed.
        let _ = self
            .runner
            .run(&["docker".into(), "rm".into(), "-f".into(), name.clone()]);

        let out =
            self.runner
                .run(&plan.docker.to_argv())
                .map_err(|e| AgentError::LaunchFailed {
                    detail: format!("{e:#}"),
                })?;
        if !out.success() {
            return Err(AgentError::LaunchFailed {
                detail: format!("docker run exited {}: {}", out.status, out.stderr.trim()),
            });
        }

        let port = plan
            .docker
            .command
            .windows(2)
            .find(|w| w[0] == "--port")
            .map(|w| w[1].clone());
        let endpoint = match port.as_deref() {
            // A worker rank is bound to port 0 and serves nothing.
            Some("0") | None => None,
            Some(p) => Some(format!("http://localhost:{p}/v1")),
        };

        Ok(Started {
            container: name,
            endpoint,
        })
    }

    fn stop(&self, recipe: &str) -> Result<(), AgentError> {
        // By LABEL, not by name. A solo launch is `atlas-{recipe}`, but a rank of
        // a cluster is `atlas-{recipe}-rank{n}` -- so stopping by name found the
        // solo container and never a rank. That is the state an operator is left
        // in when the head agent restarts while a cluster runs: the driver's
        // record is memory-only, `StopCluster` answers "this agent did not start
        // a cluster", and this path -- the only other one -- could not see the
        // containers either. Both GPUs held, no button that works, `docker rm -f`
        // on every machine by hand.
        //
        // The label is already on every container this agent starts, put there
        // by the launch plan for exactly this kind of question.
        let listed = self
            .runner
            .run(&[
                "docker".into(),
                "ps".into(),
                "-q".into(),
                "--filter".into(),
                format!("label={LABEL_RECIPE}={recipe}"),
            ])
            .map_err(|e| AgentError::LaunchFailed {
                detail: format!("{e:#}"),
            })?;
        if !listed.success() {
            return Err(AgentError::LaunchFailed {
                detail: listed.stderr.trim().to_string(),
            });
        }
        let ids: Vec<String> = listed
            .stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        if ids.is_empty() {
            // Nothing labelled. Fall back to the name so a container started by
            // an older agent, before this label existed, is still stoppable.
            let out = self
                .runner
                .run(&["docker".into(), "stop".into(), format!("atlas-{recipe}")])
                .map_err(|e| AgentError::LaunchFailed {
                    detail: format!("{e:#}"),
                })?;
            return if out.success() {
                Ok(())
            } else {
                Err(AgentError::LaunchFailed {
                    detail: out.stderr.trim().to_string(),
                })
            };
        }

        let mut argv: Vec<String> = vec!["docker".into(), "stop".into()];
        argv.extend(ids);
        let out = self
            .runner
            .run(&argv)
            .map_err(|e| AgentError::LaunchFailed {
                detail: format!("{e:#}"),
            })?;
        if out.success() {
            Ok(())
        } else {
            Err(AgentError::LaunchFailed {
                detail: out.stderr.trim().to_string(),
            })
        }
    }

    fn running(&self) -> Result<Vec<RunningLaunch>, AgentError> {
        let out = self
            .runner
            .run(&[
                "docker".into(),
                "ps".into(),
                "--filter".into(),
                format!("label={LABEL_MANAGED}=1"),
                "--format".into(),
                "{{.Names}}\t{{.Status}}".into(),
            ])
            .map_err(|e| AgentError::DockerUnavailable {
                detail: format!("{e:#}"),
            })?;
        if !out.success() {
            return Err(AgentError::DockerUnavailable {
                detail: out.stderr.trim().to_string(),
            });
        }
        Ok(out
            .stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let mut parts = line.split('\t');
                let container = parts.next().unwrap_or_default().to_string();
                let status = parts.next().unwrap_or_default().to_string();
                // Recover the recipe from the container name, which we control.
                let recipe = container
                    .strip_prefix("atlas-")
                    .and_then(|s| RecipeId::parse(s).ok());
                RunningLaunch {
                    container,
                    recipe,
                    status,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
