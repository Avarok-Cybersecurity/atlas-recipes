// SPDX-License-Identifier: AGPL-3.0-only

//! Reading the host facts a launch depends on.
//!
//! This is the only place the CLI touches the environment for launch purposes.
//! Everything downstream takes the resulting snapshot, which is what keeps
//! translation pure and reproducible.

use anyhow::Result;
use atlasctl_core::host::HostSnapshot;
use std::collections::BTreeMap;

/// Capture the current host.
pub fn snapshot() -> Result<HostSnapshot> {
    let home = atlasctl_core::platform::home_string()?;
    Ok(HostSnapshot {
        posix_user: atlasctl_core::platform::posix_user(),
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
    std::env::var("HF_HOME").unwrap_or_else(|_| {
        std::path::Path::new(home)
            .join(".cache")
            .join("huggingface")
            .display()
            .to_string()
    })
}

/// Where atlasctl keeps its configuration.
///
/// Delegates so there is one resolution order and one place that knows the
/// three files in there are a unit. See [`crate::configdir`].
///
/// # Errors
/// If no home or override is set.
pub fn config_dir() -> Result<std::path::PathBuf> {
    crate::configdir::resolve()
}

/// The config directory, checked to be usable before anyone writes to it.
///
/// Every command that mints or reads this node's identity goes through here.
/// `config_dir` alone only says where state *should* live; on a box where
/// `$HOME` belongs to another user it happily returns a path nothing can be
/// written to, and the first `load_or_create` then fails with a bare
/// `Permission denied (os error 13)` naming a file the operator never chose.
///
/// `configdir::ensure_usable` turns that into an owner/mode diagnosis and two
/// remedies. It was already wired into `agent run` and `agent install`, and
/// missing from `agent token`, `agent pair` and every `peer` verb — which is
/// most of what a new operator types, and `install.sh` tells them to run
/// `agent token` first.
///
/// # Errors
/// If the directory cannot be resolved, created, or written.
pub fn usable_config_dir() -> Result<std::path::PathBuf> {
    let dir = config_dir()?;
    crate::configdir::ensure_usable(&dir)?;
    Ok(dir)
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
    atlasctl_core::platform::home_dir()
}

/// Where atlasctl caches registry clones.
pub fn cache_dir() -> Result<std::path::PathBuf> {
    Ok(atlasctl_core::platform::cache_base()?.join("atlasctl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_reflects_the_running_process() {
        let s = snapshot().expect("a home directory is set in any sane environment");
        assert_eq!(s.posix_user, atlasctl_core::platform::posix_user());
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
