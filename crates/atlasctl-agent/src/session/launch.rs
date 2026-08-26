// SPDX-License-Identifier: AGPL-3.0-only

//! The launch half of the browser session: what the page may do to *this*
//! machine.
//!
//! The counterpart to [`super::fleet`], and split from it for the same
//! reason stated there: these verbs reach the container runtime on this box,
//! those reach another machine over the authenticated peer channel. Keeping
//! them apart is what keeps each surface small enough to read in one sitting.

use super::{Session, err};
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::{AgentError, ServerMsg};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;

impl Session<'_> {
    pub(super) fn preview(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        let recipe = match self.resolve(recipe_id) {
            Ok(r) => r,
            Err(e) => return vec![err(Some(id), e)],
        };
        let overrides = match self.check_settings(requested) {
            Ok(o) => o,
            Err(e) => return vec![err(Some(id), e)],
        };
        match self.deps.launcher.preview(&recipe, &overrides) {
            Ok(p) => vec![ServerMsg::Preview {
                id,
                command: p.command,
                unapplied: p.unapplied,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) fn launch(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        if let Err(why) = &self.deps.can_launch {
            return vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe_id.clone(),
                    reason: why.clone(),
                },
            )];
        }
        let recipe = match self.resolve(recipe_id) {
            Ok(r) => r,
            Err(e) => return vec![err(Some(id), e)],
        };
        if let Err(why) = recipe.launchable() {
            return vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe_id.clone(),
                    reason: why.to_string(),
                },
            )];
        }
        let overrides = match self.check_settings(requested) {
            Ok(o) => o,
            Err(e) => return vec![err(Some(id), e)],
        };
        match self.deps.launcher.launch(&recipe, &overrides) {
            Ok(started) => vec![ServerMsg::Started {
                id,
                recipe: recipe_id.clone(),
                container: started.container,
                endpoint: started.endpoint,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) fn stop(&mut self, id: u32, recipe_id: &RecipeId) -> Vec<ServerMsg> {
        match self.deps.launcher.stop(recipe_id.as_str()) {
            Ok(()) => vec![ServerMsg::Stopped {
                id,
                recipe: recipe_id.clone(),
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) fn status(&mut self, id: u32) -> Vec<ServerMsg> {
        match self.deps.launcher.running() {
            Ok(running) => vec![ServerMsg::Status { id, running }],
            Err(e) => vec![err(Some(id), e)],
        }
    }
}
