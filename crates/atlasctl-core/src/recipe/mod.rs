// SPDX-License-Identifier: AGPL-3.0-only

//! The recipe model: parse, validate, and stamp provenance.

mod fingerprint;
mod raw;
mod topology;

pub use raw::{EXECUTABLE_KEYS, ISOLATION_KEYS, RawRecipe};
pub use topology::Topology;

use crate::scalar::ScalarValue;
use std::collections::BTreeMap;

/// Where a recipe came from — shown wherever a recipe is displayed, so a user
/// can always tell whether what they are about to run is the vendored corpus or
/// something a remote registry supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// Compiled into this binary from the repository it was built from.
    Builtin {
        /// Path within the corpus, e.g. `qwen3.6/qwen3.6-27b-fp8.yaml`.
        path: String,
    },
    /// Supplied by a registry the user explicitly added. Never trusted.
    Remote {
        /// The registry's local name.
        registry: String,
        /// Its git URL.
        url: String,
    },
    /// Read from a path the user named directly.
    LocalPath {
        /// The file's path.
        path: String,
    },
}

/// Which serving runtime a recipe targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeKind {
    /// The Atlas engine — the only runtime atlasctl can launch.
    Atlas,
    /// Anything else, kept so the recipe can be listed and explained.
    Unsupported(String),
}

/// A parsed, validated recipe.
#[derive(Debug, Clone)]
pub struct Recipe {
    /// The recipe's name, which is its filename stem.
    pub name: String,
    /// Where it came from.
    pub provenance: Provenance,
    /// Schema version: `"2"` for current recipes, `"1"` for legacy ones.
    pub recipe_version: String,
    /// The HuggingFace model id to serve.
    pub model: String,
    /// An optional pinned revision of that model.
    pub model_revision: Option<String>,
    /// The runtime this recipe targets.
    pub runtime: RuntimeKind,
    /// The container image to run.
    pub container: String,
    /// Node bounds.
    pub topology: Topology,
    /// Serve-flag values the recipe sets.
    pub defaults: BTreeMap<String, ScalarValue>,
    /// Container environment the recipe sets.
    pub env: BTreeMap<String, String>,
    /// Human description, from the top level or from `metadata.description`.
    pub description: Option<String>,
    /// Top-level keys the schema does not name. Reported, never applied.
    pub unknown_keys: Vec<String>,
    /// Executable-content or isolation keys the recipe carries.
    ///
    /// Non-empty means the recipe is not launchable. It still loads, so it can
    /// be listed and explained — a recipe that vanishes from `recipe list` is
    /// harder to reason about than one that says why it will not run — but the
    /// keys themselves are never read again, let alone executed.
    pub refused_keys: Vec<String>,
}

/// Why a recipe could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    /// The file is not valid YAML, or does not match the recipe shape.
    #[error("{name}: could not parse recipe: {source}")]
    Parse {
        /// Recipe name.
        name: String,
        /// The underlying YAML error.
        #[source]
        source: serde_yaml_ng::Error,
    },

    /// A recipe with no `model:` names nothing to serve.
    #[error("{name}: recipe has no `model:` — there is nothing to serve")]
    MissingModel {
        /// Recipe name.
        name: String,
    },

    /// A recipe with no `container:` names no image to run.
    #[error("{name}: recipe has no `container:` — there is no image to run")]
    MissingContainer {
        /// Recipe name.
        name: String,
    },

    /// A field that lands in an argv position began with `-`.
    #[error(
        "{name}: `{field}: {value}` starts with `-`, so it would be read as an \
         option rather than a value. `RecipeId` forbids this for the same \
         reason: a leading dash lets a name be parsed as a flag."
    )]
    FlagShaped {
        /// Recipe name.
        name: String,
        /// Which field.
        field: &'static str,
        /// What it said.
        value: String,
    },
}

/// Refuse a field that would be read as an option in the argv position it
/// lands in.
fn flag_shaped(name: &str, field: &'static str, value: &str) -> Result<(), RecipeError> {
    if value.trim_start().starts_with('-') {
        return Err(RecipeError::FlagShaped {
            name: name.to_string(),
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

/// Why a loaded recipe cannot be launched.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotLaunchable {
    /// The recipe carries keys that would execute recipe-supplied code.
    #[error(
        "recipe carries executable-content keys [{}]. atlasctl never runs \
         recipe-supplied commands — that mechanism is what allowed the tool this \
         replaces to be turned against its users. Move the work into explicit \
         recipe fields, or run it yourself before launching.",
        .0.join(", ")
    )]
    ExecutableContent(Vec<String>),

    /// The recipe targets a runtime atlasctl does not drive.
    #[error("recipe targets runtime `{0}`; atlasctl launches the `atlas` runtime only")]
    ForeignRuntime(String),

    /// A legacy recipe that never declared a runtime at all.
    #[error(
        "recipe declares no `runtime:` (schema version {0}). The reference tool \
         guessed one by inspecting the command template; atlasctl does not guess \
         what to execute."
    )]
    NoRuntimeDeclared(String),
}

impl Recipe {
    /// Parse a recipe from YAML text.
    ///
    /// Pure: takes the text and its provenance, touches no filesystem.
    pub fn parse(name: &str, yaml: &str, provenance: Provenance) -> Result<Self, RecipeError> {
        let raw: RawRecipe =
            serde_yaml_ng::from_str(yaml).map_err(|source| RecipeError::Parse {
                name: name.to_string(),
                source,
            })?;

        let refused_keys: Vec<String> = raw
            .refused_keys()
            .iter()
            .map(|k| (*k).to_string())
            .collect();

        let model = raw
            .model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .ok_or_else(|| RecipeError::MissingModel {
                name: name.to_string(),
            })?;
        // `model` is rendered into `spark serve <model>` and `container` into
        // the image operand of `docker run`, both positionally and both
        // without a `--` separator ahead of them. A value beginning with `-`
        // is therefore handed to an option parser: `container: --privileged`
        // makes docker read a flag and take the NEXT token as the image. These
        // two are recipe fields, so the audited guarantee that no override
        // VALUE can become flag-shaped never covered them.
        flag_shaped(name, "model", &model)?;

        let container = raw
            .container
            .clone()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| RecipeError::MissingContainer {
                name: name.to_string(),
            })?;
        flag_shaped(name, "container", &container)?;

        let runtime = match raw.runtime.as_deref() {
            Some("atlas") => RuntimeKind::Atlas,
            Some(other) => RuntimeKind::Unsupported(other.to_string()),
            // A legacy recipe with no `runtime:` had it inferred by sniffing its
            // command template. We do not launch those, so name the gap plainly
            // rather than guessing.
            None => RuntimeKind::Unsupported(String::new()),
        };

        let description = raw.description.clone().or_else(|| {
            raw.metadata
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });

        Ok(Self {
            name: name.to_string(),
            provenance,
            recipe_version: raw.recipe_version.clone().unwrap_or_else(|| "2".into()),
            model,
            model_revision: raw.model_revision.clone(),
            runtime,
            container,
            topology: Topology::from_raw(&raw),
            defaults: raw.defaults.clone(),
            env: raw.env.clone(),
            description,
            unknown_keys: raw.unknown_keys(),
            refused_keys,
        })
    }

    /// Whether atlasctl can launch this recipe, and if not, why.
    pub fn launchable(&self) -> Result<(), NotLaunchable> {
        if !self.refused_keys.is_empty() {
            return Err(NotLaunchable::ExecutableContent(self.refused_keys.clone()));
        }
        match &self.runtime {
            RuntimeKind::Atlas => Ok(()),
            RuntimeKind::Unsupported(r) if r.is_empty() => Err(NotLaunchable::NoRuntimeDeclared(
                self.recipe_version.clone(),
            )),
            RuntimeKind::Unsupported(r) => Err(NotLaunchable::ForeignRuntime(r.clone())),
        }
    }

    /// Convenience predicate over [`Self::launchable`].
    pub fn is_launchable(&self) -> bool {
        self.launchable().is_ok()
    }

    /// The recipe's qualified name, as users type it.
    pub fn qualified_name(&self) -> String {
        match &self.provenance {
            Provenance::Builtin { .. } => format!("@atlas/{}", self.name),
            Provenance::Remote { registry, .. } => format!("@{registry}/{}", self.name),
            Provenance::LocalPath { path } => path.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
