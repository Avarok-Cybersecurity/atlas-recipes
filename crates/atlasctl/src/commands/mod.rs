// SPDX-License-Identifier: AGPL-3.0-only

//! Command implementations.

pub mod agent;
pub mod agentinfo;
pub mod agentpair;
pub mod doctor;
pub mod doctor_checks;
pub mod lifecycle;
pub mod peer;
pub mod recipe;
pub mod registry;
pub mod run;
pub mod service;

use anyhow::Result;
use atlasctl_core::io::StdFileSystem;
use atlasctl_core::registry::{RegistrySet, RemoteRegistry, RemoteStore};
use std::sync::Arc;

/// Path of the persisted remote-registry list.
pub fn registries_path() -> Result<std::path::PathBuf> {
    Ok(crate::hostinfo::config_dir()?.join("registries.yaml"))
}

/// Build the registry set: the vendored corpus, plus any remotes the user added.
///
/// A fresh install has no remotes, so this performs no network access and reads
/// no files that do not exist.
pub fn registry_set() -> Result<RegistrySet> {
    let fs = Arc::new(StdFileSystem);
    let store = RemoteStore::load(fs.as_ref(), &registries_path()?)?;
    let remotes: Vec<RemoteRegistry> = store
        .registries
        .into_iter()
        .map(|r| r.with_fs(fs.clone() as Arc<dyn atlasctl_core::io::FileSystem>))
        .collect();
    Ok(RegistrySet::with_remotes(remotes))
}
