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
use atlasctl_protocol::msg::{AgentError, ClientMsg, ServerMsg};
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
                // Every reply in this file is this machine answering its own
                // browser directly, so the provenance pair is (None, None) —
                // stated, never inferred by the page.
                on: None,
                via: None,
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
                on: None,
                via: None,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) fn stop(&mut self, id: u32, recipe_id: &RecipeId) -> Vec<ServerMsg> {
        match self.deps.launcher.stop(recipe_id.as_str()) {
            Ok(()) => vec![ServerMsg::Stopped {
                id,
                recipe: recipe_id.clone(),
                on: None,
                via: None,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) fn status(&mut self, id: u32) -> Vec<ServerMsg> {
        match self.deps.launcher.running() {
            Ok(running) => vec![ServerMsg::Status {
                id,
                running,
                on: None,
                via: None,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }
}

/// Refuse a single-node control verb aimed at another machine, or `None` for
/// anything local.
///
/// Protocol 4 carries the `on` annotation ahead of the router that honours
/// it: the session router is a later step of the forwarding design and does
/// not exist in this build. Until it does, every `Some` target — even one
/// that happens to name this machine — is refused as unroutable, typed and
/// naming the node, because executing the request here while the page
/// believes it addressed another machine is the exact misattribution the
/// provenance fields exist to prevent.
pub(super) fn refuse_forward(msg: &ClientMsg) -> Option<ServerMsg> {
    let (id, node) = match msg {
        ClientMsg::ListRecipes { id, on: Some(n) }
        | ClientMsg::Preview {
            id, on: Some(n), ..
        }
        | ClientMsg::Launch {
            id, on: Some(n), ..
        }
        | ClientMsg::Stop {
            id, on: Some(n), ..
        }
        | ClientMsg::Status { id, on: Some(n) }
        | ClientMsg::LaunchStats {
            id, on: Some(n), ..
        }
        | ClientMsg::LaunchLogs {
            id, on: Some(n), ..
        } => (*id, *n),
        _ => return None,
    };
    Some(err(
        Some(id),
        AgentError::NotRoutable {
            node,
            reason: "control forwarding is not implemented in this build; \
                     only this machine can be addressed"
                .into(),
        },
    ))
}
