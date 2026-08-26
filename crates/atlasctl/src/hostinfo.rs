// SPDX-License-Identifier: AGPL-3.0-only

//! Reading the host facts a launch depends on.
//!
//! This is the only place the CLI touches the environment for launch purposes.
//! Everything downstream takes the resulting snapshot, which is what keeps
//! translation pure and reproducible.

use anyhow::{Context, Result};
use atlasctl_core::host::HostSnapshot;
use std::collections::BTreeMap;

/// Capture the current host.
pub fn snapshot() -> Result<HostSnapshot> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(HostSnapshot {
        uid: rustix::process::getuid().as_raw(),
        gid: rustix::process::getgid().as_raw(),
        hf_cache_dir: hf_cache_dir(&home),
        home,
        env: std::env::vars().collect::<BTreeMap<_, _>>(),
    })
}

/// Where HuggingFace caches models.
///
/// `HF_HOME` wins when set, matching the reference implementation and the
/// convention every HuggingFace tool follows; otherwise the documented default
/// under the user's home directory.
fn hf_cache_dir(home: &str) -> String {
    std::env::var("HF_HOME").unwrap_or_else(|_| format!("{home}/.cache/huggingface"))
}

/// Where atlasctl keeps its configuration.
pub fn config_dir() -> Result<std::path::PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(std::path::PathBuf::from(base).join("atlasctl"))
}

/// This user's home directory.
///
/// Required, not defaulted: every path the service installer writes hangs off
/// it, and guessing would put a unit file somewhere the supervisor does not
/// look — which fails as "installed successfully, never starts".
///
/// # Errors
/// If `HOME` is not set.
pub fn home_dir() -> Result<std::path::PathBuf> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .context("HOME is not set, so there is nowhere to install a user service")
}

/// Where atlasctl caches registry clones.
pub fn cache_dir() -> Result<std::path::PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cache")))
        .context("neither XDG_CACHE_HOME nor HOME is set")?;
    Ok(std::path::PathBuf::from(base).join("atlasctl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_reflects_the_running_process() {
        let s = snapshot().expect("HOME is set in any sane environment");
        assert_eq!(s.uid, rustix::process::getuid().as_raw());
        assert!(!s.home.is_empty());
        assert!(s.hf_cache_dir.contains("huggingface"));
    }

    #[test]
    fn the_cache_path_falls_back_to_the_home_directory() {
        // Not asserting an absolute path: this must hold for any user.
        let d = cache_dir().expect("resolves");
        assert!(d.ends_with("atlasctl"));
    }
}
