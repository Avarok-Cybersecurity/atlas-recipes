// SPDX-License-Identifier: AGPL-3.0-only

//! Why the agent said no.
//!
//! A closed enum rather than a string, so every refusal the agent can produce
//! has to be named here — and adding a new one is a decision somebody makes on
//! purpose rather than a message that appears in a browser one day. The page
//! renders these, so each variant's text is written for an operator to act on,
//! not for a developer to grep.

use super::*;

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
