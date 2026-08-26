// SPDX-License-Identifier: AGPL-3.0-only

//! This machine, acting as one rank of somebody else's cluster.
//!
//! The head never renders another machine's command. It does not know which
//! recipe revision that machine has, what its flag table claims, or what
//! hardware it is exposing — so a preview the head invented would be a guess
//! presented as the thing that will execute. Each rank answers for itself,
//! through the same `translate()` the launcher uses, so preview and execution
//! cannot drift.
//!
//! ## The reservation
//!
//! `prepare` renders the command and *keeps it*. `commit` runs what was kept,
//! and takes only an epoch. So the bytes that reach the container runtime were
//! produced by this machine, from its own recipe, before the head was told
//! anything — a head compromised between the two phases can start the launch
//! the operator already previewed, or not start it, and nothing else.
//!
//! One reservation at a time, deliberately. A machine that has agreed to be
//! rank 1 of one cluster must refuse to be rank 1 of another, or the second
//! commit would silently replace the first cluster's container mid-launch.

use anyhow::{Context, Result, bail};
use atlasctl_agent::cluster::{PrepareReply, RankAssignment, RefusalReason};
use atlasctl_agent::rank::RankService;
use atlasctl_core::chain::UserConfig;
use atlasctl_core::docker::collective::CollectiveEnv;
use atlasctl_core::docker::profile::{DeviceProfile, LaunchProfile};
use atlasctl_core::docker::translate::{LaunchContext, Placement, translate};
use atlasctl_core::host::HostSnapshot;
use atlasctl_core::io::ProcessRunner;
use atlasctl_core::registry::{RecipeRef, RegistrySet};
use atlasctl_core::settings;
use std::sync::{Arc, Mutex};

/// A rendered rank, held between prepare and commit.
struct Reservation {
    epoch: String,
    recipe: String,
    plan: atlasctl_core::docker::LaunchPlan,
}

/// Serves rank requests from this machine's own recipe inventory.
pub struct LocalRankService {
    registry: RegistrySet,
    host: HostSnapshot,
    profile: &'static LaunchProfile,
    devices: Box<dyn DeviceProfile>,
    collective: Box<dyn CollectiveEnv>,
    runner: Arc<dyn ProcessRunner>,
    /// Why this machine cannot run models, when it cannot.
    can_launch: Result<(), String>,
    reserved: Mutex<Option<Reservation>>,
}

impl std::fmt::Debug for LocalRankService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRankService").finish_non_exhaustive()
    }
}

impl LocalRankService {
    /// Build a rank service for this machine.
    #[must_use]
    pub fn new(
        registry: RegistrySet,
        host: HostSnapshot,
        profile: &'static LaunchProfile,
        devices: Box<dyn DeviceProfile>,
        collective: Box<dyn CollectiveEnv>,
        runner: Arc<dyn ProcessRunner>,
        can_launch: Result<(), String>,
    ) -> Self {
        Self {
            registry,
            host,
            profile,
            devices,
            collective,
            runner,
            can_launch,
            reserved: Mutex::new(None),
        }
    }

    /// Resolve, agree on the revision, and translate — the shared path behind
    /// both preview and prepare, so a preview cannot succeed where the prepare
    /// that follows it would fail.
    fn plan_for(&self, a: &RankAssignment) -> Result<atlasctl_core::docker::LaunchPlan> {
        let recipe = self
            .registry
            .resolve(&RecipeRef::parse(&a.recipe))
            .with_context(|| format!("this node does not ship recipe {}", a.recipe))?;

        // Two nodes running different revisions of one recipe would launch two
        // different models and call it one cluster; the failure would surface
        // as wrong output rather than as an error. An empty hash means the head
        // did not state one, which is not agreement.
        let local = recipe.content_hash();
        if a.recipe_hash != local {
            bail!(
                "{}",
                RefusalReason::RecipeMismatch {
                    recipe: a.recipe.clone(),
                    head: if a.recipe_hash.is_empty() {
                        "(none sent)".to_owned()
                    } else {
                        a.recipe_hash.clone()
                    },
                    local,
                }
            );
        }

        // The wire carries settings, and they go back through the SAME bounded
        // schema the browser's settings go through. A peer cannot widen what a
        // setting may be simply by virtue of being a peer.
        let overrides = settings::validate(&a.settings).map_err(|errors| {
            anyhow::anyhow!(
                "this node will not accept those settings: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;

        let placement = Placement::Rank {
            rank: a.rank,
            world_size: a.world_size,
            master_addr: a.master_addr.clone(),
            master_port: a.master_port,
        };

        translate(
            &recipe,
            &overrides,
            &UserConfig::default(),
            &self.host,
            &placement,
            &LaunchContext {
                profile: self.profile,
                devices: self.devices.as_ref(),
                collective: self.collective.as_ref(),
            },
        )
        .with_context(|| format!("rendering rank {} of {}", a.rank, a.recipe))
    }

    /// Whether the container runtime is answering here.
    fn docker_ok(&self) -> bool {
        self.runner
            .run(&atlasctl_agent::fleet::docker_probe_argv())
            .is_ok_and(|o| o.success())
    }
}

impl RankService for LocalRankService {
    fn render(&self, assignment: &RankAssignment) -> Result<(String, Vec<String>)> {
        let plan = self.plan_for(assignment)?;
        // Surfaced rather than swallowed: a setting this node's flag table does
        // not claim will silently not apply, and the operator should see that
        // before they commit rather than wonder afterwards.
        let unmapped = plan.unmapped.iter().map(|u| u.key.clone()).collect();
        Ok((plan.docker.to_string(), unmapped))
    }

    fn content_hash(&self, recipe: &str) -> Result<String> {
        Ok(self
            .registry
            .resolve(&RecipeRef::parse(recipe))
            .with_context(|| format!("this node does not ship recipe {recipe}"))?
            .content_hash())
    }

    fn prepare(&self, epoch: &str, assignment: &RankAssignment) -> PrepareReply {
        let refuse = |r: RefusalReason| PrepareReply::Refused {
            reason: r.to_string(),
        };

        if let Err(why) = &self.can_launch {
            return refuse(RefusalReason::NotLaunchable(why.clone()));
        }

        // Checked before rendering: a machine that has agreed to be rank 1 of
        // one cluster must refuse to be rank 1 of another, or the second commit
        // would replace the first cluster's container mid-launch. Re-preparing
        // the SAME epoch is allowed, because a retried prepare after a dropped
        // connection is an ordinary thing and not a second cluster.
        {
            let held = self.reserved.lock().expect("reservation lock poisoned");
            if let Some(r) = held.as_ref()
                && r.epoch != epoch
            {
                return refuse(RefusalReason::Reserved {
                    recipe: r.recipe.clone(),
                });
            }
        }

        if !self.docker_ok() {
            return refuse(RefusalReason::DockerUnavailable);
        }

        // Rendering is the last check, and it is the strongest one: it proves
        // this machine can actually produce a command for this assignment, so
        // commit has nothing left that can fail on its own terms.
        let plan = match self.plan_for(assignment) {
            Ok(p) => p,
            Err(e) => {
                return PrepareReply::Refused {
                    reason: format!("{e:#}"),
                };
            }
        };

        *self.reserved.lock().expect("reservation lock poisoned") = Some(Reservation {
            epoch: epoch.to_owned(),
            recipe: assignment.recipe.clone(),
            plan,
        });
        PrepareReply::Prepared
    }

    fn commit(&self, epoch: &str) -> Result<String> {
        // Taken, not borrowed: a committed reservation is spent, so a replayed
        // commit frame starts nothing a second time.
        let held = {
            let mut slot = self.reserved.lock().expect("reservation lock poisoned");
            match slot.as_ref() {
                Some(r) if r.epoch == epoch => slot.take().expect("just matched"),
                Some(r) => bail!(
                    "this node is holding a reservation for {}, not {epoch}",
                    r.epoch
                ),
                None => bail!("this node has no reservation for {epoch}; prepare first"),
            }
        };

        let name = held.plan.docker.name.clone();
        // Clear a previous container of the same name. Failure is expected when
        // nothing is there; the run below is what has to succeed.
        let _ = self
            .runner
            .run(&["docker".into(), "rm".into(), "-f".into(), name.clone()]);

        let out = self
            .runner
            .run(&held.plan.docker.to_argv())
            .context("starting the rank")?;
        if !out.success() {
            bail!("docker run exited {}: {}", out.status, out.stderr.trim());
        }
        Ok(name)
    }

    fn alive(&self, container: &str) -> Result<bool> {
        let out = self
            .runner
            .run(&[
                "docker".into(),
                "inspect".into(),
                "-f".into(),
                "{{.State.Running}}".into(),
                container.to_owned(),
            ])
            .context("asking the container runtime about a rank")?;
        // A container that has already been removed is not running, and that is
        // an answer rather than a failure: a rank that died under `--rm` is
        // exactly the case this exists to catch.
        if !out.success() {
            return Ok(false);
        }
        Ok(out.stdout.trim() == "true")
    }

    fn stop(&self, container: &str) -> Result<()> {
        let out = self
            .runner
            .run(&[
                "docker".into(),
                "rm".into(),
                "-f".into(),
                container.to_owned(),
            ])
            .context("stopping the rank")?;
        // Already gone is the outcome the caller wanted, so it is not a
        // failure: rollback asking about a container that never started is an
        // ordinary race.
        if !out.success() && !out.stderr.contains("No such container") {
            bail!("docker rm exited {}: {}", out.status, out.stderr.trim());
        }
        Ok(())
    }

    fn abort(&self, epoch: &str) {
        let mut slot = self.reserved.lock().expect("reservation lock poisoned");
        // Only the named reservation: an abort for a stale epoch arriving late
        // must not release a reservation made since.
        if slot.as_ref().is_some_and(|r| r.epoch == epoch) {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests;
