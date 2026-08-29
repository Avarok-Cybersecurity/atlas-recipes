// SPDX-License-Identifier: AGPL-3.0-only

//! The launch half of the browser session: what the page may do to *this*
//! machine.
//!
//! The counterpart to [`super::fleet`], and split from it for the same
//! reason stated there: these verbs reach the container runtime on this box,
//! those reach another machine over the authenticated peer channel. Keeping
//! them apart is what keeps each surface small enough to read in one sitting.
//!
//! Every verb here delegates to [`crate::control::LocalControl`] — the same
//! core the peer channel's terminal `Control` handler executes through — and
//! keeps for itself only what is session-shaped: the correlation `id`, the
//! `ServerMsg` envelope, and the denied-attempt log.

use super::{Session, err};
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::ServerMsg;
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;

impl Session<'_> {
    pub(super) fn preview(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        match self.control().preview(recipe_id, requested) {
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
            Err(e) => {
                self.note_denied(&e);
                vec![err(Some(id), e)]
            }
        }
    }

    pub(super) async fn launch(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        match self.control().launch(recipe_id, requested).await {
            Ok(started) => vec![ServerMsg::Started {
                id,
                recipe: recipe_id.clone(),
                container: started.container,
                endpoint: started.endpoint,
                on: None,
                via: None,
            }],
            Err(e) => {
                self.note_denied(&e);
                vec![err(Some(id), e)]
            }
        }
    }

    pub(super) async fn stop(&mut self, id: u32, recipe_id: &RecipeId) -> Vec<ServerMsg> {
        match self.control().stop(recipe_id).await {
            Ok(()) => vec![ServerMsg::Stopped {
                id,
                recipe: recipe_id.clone(),
                on: None,
                via: None,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    pub(super) async fn status(&mut self, id: u32) -> Vec<ServerMsg> {
        match self.control().status().await {
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
