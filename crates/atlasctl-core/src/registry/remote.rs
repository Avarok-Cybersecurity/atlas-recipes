// SPDX-License-Identifier: AGPL-3.0-only

//! Remote registries: opt-in, and never trusted.
//!
//! A remote registry supplies recipe *data* and nothing else. There is no
//! `trusted` field in this module or in the config it reads — the mechanism
//! does not exist, so no configuration edit and no upstream change can enable
//! it. Recipes from a remote pass through exactly the same loader as built-in
//! ones, which refuses executable-content keys outright.

use crate::io::FileSystem;
use crate::recipe::{Provenance, Recipe, RecipeError};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A registry the user added explicitly.
#[derive(Clone, Serialize, Deserialize)]
pub struct RemoteRegistry {
    /// Local name, used in `@name/recipe`.
    pub name: String,
    /// Git URL it was cloned from.
    pub url: String,
    /// Subdirectory within the clone that holds recipes.
    #[serde(default = "default_subpath")]
    pub subpath: String,
    /// Local clone path.
    pub path: PathBuf,

    #[serde(skip)]
    fs: Option<Arc<dyn FileSystem>>,
}

impl std::fmt::Debug for RemoteRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The attached filesystem is machinery, not data; showing it would make
        // every error message noisier without saying anything useful.
        f.debug_struct("RemoteRegistry")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("subpath", &self.subpath)
            .field("path", &self.path)
            .finish()
    }
}

fn default_subpath() -> String {
    "recipes".to_string()
}

impl PartialEq for RemoteRegistry {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.url == other.url && self.path == other.path
    }
}

impl RemoteRegistry {
    /// Attach the filesystem this registry reads through.
    pub fn with_fs(mut self, fs: Arc<dyn FileSystem>) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Directory holding this registry's recipe files.
    pub fn recipe_dir(&self) -> PathBuf {
        self.path.join(&self.subpath)
    }

    /// Recipe names this registry offers.
    pub fn recipe_names(&self) -> Vec<String> {
        let Some(fs) = &self.fs else {
            return Vec::new();
        };
        fs.list_files(&self.recipe_dir(), "yaml")
            .unwrap_or_default()
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
            .collect()
    }

    /// Load one recipe by name.
    pub fn load(&self, name: &str) -> Option<Result<Recipe, RecipeError>> {
        let fs = self.fs.as_ref()?;
        // The name is joined onto a directory, so it decides which file is
        // read. `RecipeRef::parse` accepts anything after the `@registry/`,
        // including `../../..`, and one caller — `RankAssignment.recipe` — is
        // an unvalidated string from whichever node is acting as head. Without
        // this, a scoped ref could walk out of the cache and have any `*.yaml`
        // on the box read and parsed as a recipe.
        if !is_safe_recipe_name(name) {
            return None;
        }
        let path = self.recipe_dir().join(format!("{name}.yaml"));
        let yaml = fs.read_to_string(&path).ok()?;
        Some(Recipe::parse(
            name,
            &yaml,
            Provenance::Remote {
                registry: self.name.clone(),
                url: self.url.clone(),
            },
        ))
    }
}

/// Argv to clone a registry.
///
/// The bare `--` matters: without it a URL beginning with a dash would be
/// parsed by git as an option. It costs one argument and removes a class of
/// argument-injection entirely.
pub fn git_clone_argv(url: &str, dest: &Path) -> Vec<String> {
    vec![
        "git".into(),
        "clone".into(),
        "--depth".into(),
        "1".into(),
        "--".into(),
        url.to_string(),
        dest.display().to_string(),
    ]
}

/// Argv to update an existing clone, as a fetch followed by a hard reset.
///
/// The reset is destructive, so callers must confirm the path is inside our own
/// cache directory first — see [`RemoteStore::guard_cache_path`].
pub fn git_update_argv(dest: &Path) -> Vec<Vec<String>> {
    let at = |args: &[&str]| {
        let mut v = vec![
            "git".to_string(),
            "-C".to_string(),
            dest.display().to_string(),
        ];
        v.extend(args.iter().map(|s| (*s).to_string()));
        v
    };
    vec![
        at(&["fetch", "--depth", "1", "origin"]),
        at(&["reset", "--hard", "FETCH_HEAD"]),
    ]
}

/// The persisted list of remote registries.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RemoteStore {
    /// Registries the user has added.
    #[serde(default)]
    pub registries: Vec<RemoteRegistry>,
}

impl RemoteStore {
    /// Load the store, treating an absent file as an empty store.
    pub fn load(fs: &dyn FileSystem, path: &Path) -> Result<Self> {
        if !fs.exists(path) {
            return Ok(Self::default());
        }
        Ok(serde_yaml_ng::from_str(&fs.read_to_string(path)?)?)
    }

    /// Persist the store.
    pub fn save(&self, fs: &dyn FileSystem, path: &Path) -> Result<()> {
        fs.write_atomic(path, &serde_yaml_ng::to_string(self)?)
    }

    /// Add a registry, rejecting reserved and duplicate names.
    pub fn add(&mut self, registry: RemoteRegistry) -> Result<()> {
        if super::RESERVED.contains(&registry.name.as_str()) {
            bail!(
                "`{}` is a reserved registry name and cannot be added",
                registry.name
            );
        }
        if self.registries.iter().any(|r| r.name == registry.name) {
            bail!("a registry named `{}` is already configured", registry.name);
        }
        self.registries.push(registry);
        Ok(())
    }

    /// Remove a registry by name, reporting whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.registries.len();
        self.registries.retain(|r| r.name != name);
        self.registries.len() != before
    }

    /// Refuse to operate destructively on anything outside our cache.
    ///
    /// `git reset --hard` inside a directory the user chose would be an easy
    /// way to destroy their work, so the update path is confined by
    /// construction rather than by care.
    ///
    /// `starts_with` alone is not that confinement. It compares COMPONENTS, and
    /// `<cache>/../../../my-project` begins with `<cache>` by that measure — so
    /// the guard returned ok for a path that resolves to the user's own work,
    /// and the hard reset followed. `path` is deserialised straight out of
    /// `registries.yaml`, so nothing upstream had rejected the `..` either.
    ///
    /// Both halves are needed. The lexical check catches a path that does not
    /// exist yet, where `canonicalize` cannot answer at all; canonicalising
    /// catches a symlink, which no amount of component inspection can see.
    ///
    /// # Errors
    /// If the target is outside the cache, or cannot be resolved to decide.
    pub fn guard_cache_path(cache_root: &Path, target: &Path) -> Result<()> {
        let refuse = |resolved: &Path| -> anyhow::Error {
            anyhow::anyhow!(
                "refusing to update {}: it is outside the registry cache at {}",
                resolved.display(),
                cache_root.display()
            )
        };

        if target
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(refuse(target));
        }
        if !target.starts_with(cache_root) {
            return Err(refuse(target));
        }

        // A registry being updated has been cloned, so it exists and this
        // resolves. When it does not, the lexical checks above already stand;
        // failing to resolve is not treated as permission.
        if let (Ok(real_target), Ok(real_root)) = (target.canonicalize(), cache_root.canonicalize())
            && !real_target.starts_with(&real_root)
        {
            return Err(refuse(&real_target));
        }
        Ok(())
    }
}

/// Whether a scoped name may be joined onto the cache directory.
///
/// A remote name is `family/leaf`, so slashes are legitimate and cannot simply
/// be banned. Each SEGMENT must be a valid `RecipeId` instead — the alphabet
/// that already exists for exactly this reason, whose own doc says "`..` and
/// `/` would let it escape a directory". `..` fails it because an id must
/// start with a letter or digit.
pub(crate) fn is_safe_recipe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .split('/')
            .all(|seg| atlasctl_protocol::RecipeId::parse(seg).is_ok())
}
