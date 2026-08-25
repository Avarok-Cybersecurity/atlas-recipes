// SPDX-License-Identifier: AGPL-3.0-only

//! The message catalogue.
//!
//! Internally tagged on `type`, which maps directly onto a TypeScript
//! discriminated union so the browser narrows on `msg.type` without unwrapping
//! a nesting level.
//!
//! The set of client messages *is* the capability surface. There is no relay,
//! no forward, and no raw-command verb, and the enum is closed — an unknown
//! `type` fails deserialization rather than reaching a handler. That is what
//! keeps the agent from becoming an open proxy for whatever page is talking
//! to it.

use crate::id::RecipeId;
use crate::settings::{SettingError, SettingSpec, SettingValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a client may ask for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame. Anything else before this is refused.
    Hello {
        /// Protocol version the client chose.
        protocol_version: u32,
        /// Pairing token, proving the user connected this browser deliberately.
        token: String,
    },

    /// Ask for the recipe inventory again.
    ListRecipes {
        /// Correlates the reply.
        id: u32,
    },

    /// Render the command a launch would run, without running it.
    Preview {
        /// Correlates the reply.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },

    /// Start a recipe.
    Launch {
        /// Correlates the reply.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings, validated against the schema before use.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },

    /// Stop a running launch.
    Stop {
        /// Correlates the reply.
        id: u32,
        /// Which recipe to stop.
        recipe: RecipeId,
    },

    /// Ask what is running.
    Status {
        /// Correlates the reply.
        id: u32,
    },
}

/// What the agent says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent unsolicited as soon as the connection opens, before authentication,
    /// so a client can report a version mismatch rather than timing out.
    Welcome {
        /// Lowest protocol version this agent speaks.
        protocol_min: u32,
        /// Highest protocol version this agent speaks.
        protocol_max: u32,
        /// Agent version, for display.
        agent_version: String,
    },

    /// Authentication succeeded; here is everything the client needs.
    Ready {
        /// Negotiated protocol version.
        protocol_version: u32,
        /// The full settings schema, so the client renders what we validate.
        schema: Vec<SettingSpec>,
        /// Every recipe this agent can launch.
        recipes: Vec<RecipeInfo>,
        /// Whether this machine can actually run a recipe.
        can_launch: bool,
        /// Why not, when it cannot.
        can_launch_reason: Option<String>,
    },

    /// Reply to `ListRecipes`.
    Recipes {
        /// Correlation id.
        id: u32,
        /// The inventory.
        recipes: Vec<RecipeInfo>,
    },

    /// Reply to `Preview`.
    Preview {
        /// Correlation id.
        id: u32,
        /// The exact command, shell-quoted for display.
        command: String,
        /// Settings the recipe carries that this agent does not understand.
        ///
        /// Surfaced rather than dropped: the tool this replaces discarded these
        /// silently, which is how a stated correctness pin went unapplied for
        /// months without anyone noticing.
        unapplied: Vec<String>,
    },

    /// A launch was accepted and started.
    Started {
        /// Correlation id.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// The container name, for logs and stop.
        container: String,
        /// Where the model is served, when it serves.
        endpoint: Option<String>,
    },

    /// Reply to `Status`.
    Status {
        /// Correlation id.
        id: u32,
        /// What is running.
        running: Vec<RunningLaunch>,
    },

    /// A launch was stopped.
    Stopped {
        /// Correlation id.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
    },

    /// Something failed.
    Error {
        /// Correlation id, when the failure answers a request.
        id: Option<u32>,
        /// What went wrong.
        error: AgentError,
    },
}

/// One recipe, as the client sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeInfo {
    /// Stable identifier.
    pub id: RecipeId,
    /// Model this recipe serves.
    pub model: String,
    /// Nodes it needs.
    pub nodes: u32,
    /// Whether this agent can launch it.
    pub runnable: bool,
    /// Why not, when it cannot.
    pub reason: Option<String>,
    /// Settings the recipe sets, as launch defaults.
    pub defaults: BTreeMap<String, SettingValue>,
}

/// A launch currently running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunningLaunch {
    /// Container name.
    pub container: String,
    /// Recipe that produced it, when it is one of ours.
    pub recipe: Option<RecipeId>,
    /// Docker's status line.
    pub status: String,
}

/// Everything that can go wrong, typed so a client can react rather than
/// pattern-match on prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum AgentError {
    /// The client's protocol version is not one we speak.
    #[error("this agent speaks protocol {min}..={max}, the client asked for {requested}")]
    UnsupportedProtocol {
        /// Lowest supported.
        min: u32,
        /// Highest supported.
        max: u32,
        /// What was asked for.
        requested: u32,
    },

    /// The pairing token was absent or wrong.
    #[error("pairing token rejected — run `atlasctl agent token` and paste it into the page")]
    NotPaired,

    /// A frame arrived before the handshake completed.
    #[error("expected a hello frame first")]
    NotReady,

    /// The frame did not deserialize.
    #[error("malformed message: {detail}")]
    InvalidMessage {
        /// What was wrong with it.
        detail: String,
    },

    /// No such recipe in the compiled-in set.
    #[error("no recipe named `{recipe}`")]
    UnknownRecipe {
        /// What was asked for.
        recipe: String,
    },

    /// The recipe exists but cannot be launched here.
    #[error("`{recipe}` cannot be launched: {reason}")]
    NotLaunchable {
        /// Which recipe.
        recipe: RecipeId,
        /// Why not.
        reason: String,
    },

    /// One or more settings were rejected.
    #[error("{} setting(s) rejected", .errors.len())]
    BadSettings {
        /// Every problem at once.
        errors: Vec<SettingError>,
    },

    /// Something is already running.
    #[error("`{recipe}` is already running")]
    AlreadyRunning {
        /// Which recipe.
        recipe: RecipeId,
    },

    /// Docker is not usable.
    #[error("docker is not available: {detail}")]
    DockerUnavailable {
        /// What the probe said.
        detail: String,
    },

    /// The launch itself failed.
    #[error("launch failed: {detail}")]
    LaunchFailed {
        /// What went wrong.
        detail: String,
    },
}

#[cfg(test)]
mod tests;
