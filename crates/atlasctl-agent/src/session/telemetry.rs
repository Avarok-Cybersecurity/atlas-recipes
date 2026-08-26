// SPDX-License-Identifier: AGPL-3.0-only

//! What the page may ask about a launch that is already running.

use super::{Session, err};
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::{AgentError, ServerMsg};

/// Sampling a running launch.
///
/// A trait so the session stays I/O-free: sampling means opening a socket to
/// whatever the model is serving on, and a session that did that itself could
/// not be tested without a model.
pub trait LaunchTelemetry: Send + Sync {
    /// Take a reading of a running launch.
    ///
    /// # Errors
    /// If the model is not answering — the ordinary state of one still loading
    /// its weights, which the page shows as "loading" rather than as a fault.
    fn sample(&self, recipe: &RecipeId) -> Result<atlasctl_protocol::msg::LaunchReading, String>;
}

impl Session<'_> {
    /// How a running launch is doing.
    pub(super) fn launch_stats(&self, id: u32, recipe_id: &RecipeId) -> Vec<ServerMsg> {
        let Some(telemetry) = self.deps.telemetry else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match telemetry.sample(recipe_id) {
            Ok(stats) => vec![ServerMsg::Stats {
                id,
                recipe: recipe_id.clone(),
                stats,
            }],
            // NotLaunchable rather than LaunchFailed: a model that has not
            // finished loading is not answering yet, and calling that a launch
            // failure would send an operator looking for a crash.
            Err(reason) => vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe_id.clone(),
                    reason,
                },
            )],
        }
    }
}
