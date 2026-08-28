// SPDX-License-Identifier: AGPL-3.0-only

//! The handful of facts that differ per operating system.
//!
//! Every one of these was a bare `std::env::var("HOME")` scattered through the
//! CLI. Collected here because they are not independent: the config directory,
//! the cache directory and the home directory must agree about which user this
//! is, and a port that fixes them one call site at a time gets that wrong in a
//! way nothing fails on until a node comes back as a stranger to its own fleet.
//!
//! # Windows
//!
//! State goes under `%LOCALAPPDATA%`, never `%APPDATA%`. The difference is
//! roaming: `%APPDATA%` follows a user between machines on a domain, and
//! `agent.key` is *this machine's* identity. Roaming it would give two machines
//! the same node identity — the same failure as sharing a private key, arrived
//! at by copying a directory.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// This user's home directory.
///
/// # Errors
/// If the platform's home variable is unset.
pub fn home_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(home_var()?))
}

/// This user's home directory, as the string a rendered command embeds.
///
/// # Errors
/// If the platform's home variable is unset.
pub fn home_string() -> Result<String> {
    home_var()
}

#[cfg(unix)]
fn home_var() -> Result<String> {
    std::env::var("HOME").context("HOME is not set, so there is nowhere to keep this node's state")
}

#[cfg(windows)]
fn home_var() -> Result<String> {
    // USERPROFILE, not HOMEDRIVE+HOMEPATH: the latter pair is a legacy of
    // network home directories and is routinely set to a share that is not
    // mounted, which fails as "installed, then cannot write its own key".
    std::env::var("USERPROFILE")
        .context("USERPROFILE is not set, so there is nowhere to keep this node's state")
}

/// The directory under which per-user configuration lives.
///
/// Returns the *base*; callers append `atlasctl`.
///
/// # Errors
/// If neither the platform override nor the home variable is set.
pub fn config_base() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join(".config"))
    }
    #[cfg(windows)]
    {
        // Deliberately not honouring XDG_CONFIG_HOME here. It is set on Windows
        // only by ports of unix tools, and honouring it would move `agent.key`
        // for a reason the operator never connected to this program.
        if let Ok(x) = std::env::var("LOCALAPPDATA")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join("AppData").join("Local"))
    }
}

/// The directory under which per-user caches live.
///
/// Returns the *base*; callers append `atlasctl`.
///
/// # Errors
/// If neither the platform override nor the home variable is set.
pub fn cache_base() -> Result<PathBuf> {
    #[cfg(unix)]
    {
        if let Ok(x) = std::env::var("XDG_CACHE_HOME")
            && !x.trim().is_empty()
        {
            return Ok(PathBuf::from(x));
        }
        Ok(home_dir()?.join(".cache"))
    }
    #[cfg(windows)]
    {
        config_base()
    }
}

/// This process's POSIX identity, if the platform has one.
///
/// `None` on Windows, and that is the answer rather than a gap: a Windows
/// account has no uid, `--user 0:0` would run a container as root, and the
/// `/etc/passwd` bind mounts that accompany `--user` name host paths that do
/// not exist there. Omitting the flag runs the image's own user, which is what
/// every other Docker-on-Windows workflow does.
#[must_use]
pub fn posix_user() -> Option<crate::host::PosixUser> {
    #[cfg(unix)]
    {
        Some(crate::host::PosixUser {
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
        })
    }
    #[cfg(windows)]
    {
        None
    }
}

/// This machine's name, for display and for a peer's `Hello`.
///
/// Never fatal: a node with an unreadable hostname is still a usable node, and
/// the fingerprint is what identifies it. The fallback is deliberately
/// recognisable rather than plausible.
#[must_use]
pub fn hostname() -> String {
    #[cfg(unix)]
    let from_os = std::fs::read_to_string("/proc/sys/kernel/hostname").ok();
    #[cfg(windows)]
    let from_os = std::env::var("COMPUTERNAME").ok();

    from_os
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown-host".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bases must differ from the home directory itself: state written
    /// straight into `$HOME` is state nothing cleans up and nothing expects.
    #[test]
    fn the_bases_are_below_the_home_directory_not_equal_to_it() {
        let home = home_dir().expect("a home directory");
        for base in [
            config_base().expect("config base"),
            cache_base().expect("cache base"),
        ] {
            assert_ne!(base, home, "state must not land directly in $HOME");
        }
    }

    /// A hostname is used in display and in a peer Hello, so an empty string
    /// would render as a nameless node rather than as a problem.
    #[test]
    fn a_hostname_is_never_empty() {
        assert!(!hostname().is_empty());
    }

    /// The uid model is a property of the platform, not of the machine: a unix
    /// build must always have one, and a Windows build must never claim one.
    #[test]
    fn the_posix_identity_matches_the_platform() {
        assert_eq!(posix_user().is_some(), cfg!(unix));
    }
}
