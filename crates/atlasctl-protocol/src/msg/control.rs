// SPDX-License-Identifier: AGPL-3.0-only

//! The forwardable control vocabulary.
//!
//! A **closed enum, deliberately not [`ClientMsg`]**. Embedding `ClientMsg` in
//! a peer frame would carry `Hello`, the pairing verbs, and the cluster verbs
//! across the relay — the open-proxy shape `msg.rs` forbids. This pair can
//! express exactly the seven single-node launch operations and nothing else,
//! so "never carries pairing frames" and "one hop, never nested" are
//! properties of the schema: there is no variant a filter could miss.
//!
//! [`ClientMsg`]: super::ClientMsg

use super::{AgentError, LaunchReading, RecipeInfo, RunningLaunch};
use crate::fleet::NodeId;
use crate::id::RecipeId;
use crate::settings::SettingValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What one agent may ask another to do on the target's own hardware.
///
/// Mirrors the browser's local launch surface verb for verb, so the target
/// executes it through the same `LocalControl` core its own browser session
/// uses — a second, less-checked execution path is how a relayed launch skips
/// schema validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlReq {
    /// The target's recipe inventory.
    ListRecipes,
    /// Render the command a launch would run, without running it. The target
    /// renders from its OWN vendored recipe; no argv crosses the wire, the
    /// same bound the cluster surface enforces.
    Preview {
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },
    /// Start a recipe on the target.
    Launch {
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings, validated by the TARGET against its own schema
        /// before use.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },
    /// Stop a running launch on the target, by recipe.
    ///
    /// DELIBERATELY WIDER than `PeerFrame::StopRank`, which names a container
    /// "so a head can only stop what it was told about". This verb can stop a
    /// launch the target's operator started locally. That widening is exactly
    /// what the `controller` grant consents to (see `Pin::controller`): a
    /// peer without the grant cannot reach this verb at all, and the grant's
    /// documented meaning is "may drive my launch surface as my own browser
    /// does". Scope stays bounded to launcher-managed recipes — a workload
    /// the launcher did not start is not addressable, because `LocalControl`
    /// resolves recipes, never containers.
    Stop {
        /// Which recipe to stop.
        recipe: RecipeId,
    },
    /// What is running on the target.
    Status,
    /// Telemetry for a running launch on the target.
    Stats {
        /// Which launch.
        recipe: RecipeId,
    },
    /// Log tail for a running launch on the target. The TARGET enforces the
    /// line cap, exactly as the local `LaunchLogs` does — a relay-supplied
    /// cap would let a requester bypass the target's own bound.
    Logs {
        /// Which launch.
        recipe: RecipeId,
        /// How many lines to return, capped by the target.
        lines: u32,
    },
}

/// The answer to a [`ControlReq`].
///
/// Each success variant carries exactly the payload fields of the
/// corresponding [`ServerMsg`] reply (`Recipes`, `Preview`, `Started`,
/// `Stopped`, `Status`, `Stats`, `Logs`) minus the correlation `id`, reusing
/// the same payload structs ([`RecipeInfo`], [`RunningLaunch`],
/// [`LaunchReading`]) — one shape on both surfaces means nothing drifts
/// (SSOT).
///
/// [`ServerMsg`]: super::ServerMsg
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlRep {
    /// The target's inventory.
    Recipes {
        /// The inventory.
        recipes: Vec<RecipeInfo>,
    },
    /// The command a launch would run on the target.
    Previewed {
        /// The exact command, shell-quoted for display.
        command: String,
        /// Settings the recipe carries that the target does not understand.
        ///
        /// Surfaced rather than dropped, exactly as the local reply does: the
        /// tool this replaces discarded these silently, which is how a stated
        /// correctness pin went unapplied for months without anyone noticing.
        unapplied: Vec<String>,
    },
    /// A launch was accepted and started on the target.
    Started {
        /// Which recipe.
        recipe: RecipeId,
        /// The container name, for logs and stop.
        container: String,
        /// Where the model is served, when it serves.
        endpoint: Option<String>,
    },
    /// A launch was stopped on the target.
    Stopped {
        /// Which recipe.
        recipe: RecipeId,
    },
    /// What is running on the target.
    Status {
        /// What is running.
        running: Vec<RunningLaunch>,
    },
    /// How a running launch on the target is doing.
    Stats {
        /// Which launch.
        recipe: RecipeId,
        /// The reading. Every field is optional: absent means the engine does
        /// not report it, or that there is not yet a second sample to
        /// difference against — never zero.
        stats: LaunchReading,
    },
    /// The tail of a launch's log on the target.
    Logs {
        /// Which launch.
        recipe: RecipeId,
        /// The container the lines came from, so an operator can go read more.
        container: String,
        /// Lines, oldest first. Sanitised of control characters by the target,
        /// because a log line is attacker-influenced text that a browser is
        /// about to render.
        lines: Vec<String>,
        /// Whether the container is still running. A tail from a container
        /// that has exited is the last thing it said, not the latest news.
        running: bool,
    },
    /// Someone in the chain said no.
    ///
    /// `by` names WHO refused, because "dgx1 could not reach dgx3" and "dgx3
    /// said no" send the operator to different machines. A refusal that does
    /// not say whose it is teaches the operator to restart the wrong box.
    Refused {
        /// Who refused.
        by: NodeId,
        /// Why.
        error: AgentError,
    },
}
