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
    pub fn guard_cache_path(cache_root: &Path, target: &Path) -> Result<()> {
        if !target.starts_with(cache_root) {
            bail!(
                "refusing to update {}: it is outside the registry cache at {}",
                target.display(),
                cache_root.display()
            );
        }
        Ok(())
    }
}
