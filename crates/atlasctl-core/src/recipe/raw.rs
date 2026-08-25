// SPDX-License-Identifier: AGPL-3.0-only

//! The on-disk recipe shape, exactly as YAML presents it.
//!
//! This layer is deliberately permissive: it deserializes without judgement so
//! that the conversion in [`super`] can produce good errors and warnings rather
//! than a bare serde message. Every field is optional here; requiredness is
//! enforced during conversion.

use crate::scalar::ScalarValue;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Keys that carry executable content in the reference implementation.
///
/// These are the reason this project exists. In sparkrun, `post_commands` ran
/// on the host via `subprocess.run(shell=True)`, while `pre_exec`, `post_exec`
/// and `mods` ran as root inside the container — all reachable from a registry
/// that the tool marked "trusted" by default. atlasctl refuses to load a recipe
/// carrying any of them rather than ignoring them, so a recipe written for the
/// old tool cannot quietly lose a step it depended on.
pub const EXECUTABLE_KEYS: [&str; 7] = [
    "pre_exec",
    "post_exec",
    "post_commands",
    "stop_after_post",
    "mods",
    "builder",
    "builder_config",
];

/// Keys that alter container isolation, which only the launch profile may set.
pub const ISOLATION_KEYS: [&str; 1] = ["executor_config"];

/// A recipe as parsed, before validation or normalization.
#[derive(Debug, Clone, Deserialize)]
pub struct RawRecipe {
    #[serde(default)]
    pub recipe_version: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_revision: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub runtime_version: Option<String>,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub min_nodes: Option<u32>,
    #[serde(default)]
    pub max_nodes: Option<u32>,
    #[serde(default)]
    pub solo_only: Option<bool>,
    #[serde(default)]
    pub cluster_only: Option<bool>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub defaults: BTreeMap<String, ScalarValue>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_yaml_ng::Value>,

    /// Everything the schema does not name.
    ///
    /// The reference implementation swept unknown top-level keys into its
    /// runtime config, where they could silently influence a launch. Here they
    /// are collected only so they can be *reported*; they never reach a command.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

impl RawRecipe {
    /// Names of any executable-content or isolation keys this recipe carries.
    pub fn refused_keys(&self) -> Vec<&'static str> {
        EXECUTABLE_KEYS
            .iter()
            .chain(ISOLATION_KEYS.iter())
            .filter(|k| self.extra.contains_key(**k))
            .copied()
            .collect()
    }

    /// Unknown top-level keys, excluding the ones we refuse outright.
    pub fn unknown_keys(&self) -> Vec<String> {
        let refused = self.refused_keys();
        self.extra
            .keys()
            .filter(|k| !refused.contains(&k.as_str()))
            .cloned()
            .collect()
    }
}
