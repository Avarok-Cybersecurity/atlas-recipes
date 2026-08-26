// SPDX-License-Identifier: AGPL-3.0-only

//! Where atlasctl keeps its state, and what to say when it cannot.
//!
//! Three files live here and they are **one unit**: `browser.token` pairs a
//! browser, `agent.key` is this node's identity, and `peers.json` is who it
//! trusts. Splitting them is never useful and is actively harmful — moving the
//! directory to dodge a permission problem silently reissues the identity and
//! drops every pin, so the node comes back as a stranger to its own fleet.
//! That is why the override below relocates all three together and why
//! `XDG_CONFIG_HOME` is documented as the wrong tool for the job.
//!
//! The diagnosis half exists because the failure this replaces was
//! `Permission denied (os error 13)` and nothing else. That is true and
//! useless: it names neither the reason (a directory owned by another user)
//! nor either remedy. An error an operator cannot act on is a bug report they
//! have to write instead.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Environment variable that relocates the whole config directory.
pub const DIR_ENV: &str = "ATLASCTL_CONFIG_DIR";

/// Resolve the configuration directory.
///
/// In order: an explicit override, then `XDG_CONFIG_HOME`, then `HOME`. The
/// override is taken **verbatim** — the operator named a directory, so
/// appending `atlasctl` to it would put the state somewhere they did not ask
/// for and did not expect to have to look.
///
/// # Errors
/// If none of the three are set.
pub fn resolve() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var(DIR_ENV)
        && !dir.trim().is_empty()
    {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.config")))
        .with_context(|| {
            format!("neither {DIR_ENV}, XDG_CONFIG_HOME nor HOME is set, so there is nowhere to keep this node's identity")
        })?;
    Ok(PathBuf::from(base).join("atlasctl"))
}

/// What a directory looks like to the process trying to use it.
///
/// Split out so the advice below is a pure function of facts rather than of a
/// filesystem, and can therefore be tested for the case that actually happened
/// without needing another user account to reproduce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirFacts {
    /// Who owns it.
    pub owner_uid: u32,
    /// Its permission bits.
    pub mode: u32,
    /// Who is asking.
    pub current_uid: u32,
}

/// Why a directory cannot be written, in words an operator can act on.
///
/// `None` when nothing is wrong with it.
#[must_use]
pub fn advice(dir: &Path, f: DirFacts) -> Option<String> {
    let shown = dir.display();
    if f.owner_uid != f.current_uid {
        return Some(format!(
            "{shown} is owned by uid {} but this process is uid {}.\n\
             This usually means it was first created by a different user — \
             `sudo atlasctl …`, or a service running as root.\n\
             Either take it back:\n    \
             sudo chown -R {} {shown}\n\
             or keep this node's state somewhere else:\n    \
             atlasctl agent run --config-dir ~/.atlasctl\n\
             Do not use XDG_CONFIG_HOME for this: it moves agent.key too, so \
             the node gets a new identity and loses every peer it had paired.",
            f.owner_uid, f.current_uid, f.current_uid
        ));
    }
    // Owned by us but still not writable: the owner write bit is off.
    if f.mode & 0o200 == 0 {
        return Some(format!(
            "{shown} is yours but not writable (mode {:04o}).\n\
             Restore it with:\n    chmod u+rwx {shown}",
            f.mode & 0o7777
        ));
    }
    None
}

/// Make sure the configuration directory exists and this process can write it.
///
/// Called before anything reads or writes state, so a permission problem is
/// reported once, in full, rather than as whichever of the three files happened
/// to be touched first.
///
/// # Errors
/// If the directory cannot be created, or exists but is not writable.
pub fn ensure_usable(dir: &Path) -> Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).with_context(|| {
            format!(
                "creating {}. The parent directory is probably not writable by this user.",
                dir.display()
            )
        })?;
        return Ok(());
    }
    if !dir.is_dir() {
        bail!("{} exists but is not a directory", dir.display());
    }

    let facts = facts(dir)?;
    if let Some(why) = advice(dir, facts) {
        bail!("{why}");
    }

    // Owner and mode can both look fine and the write can still fail — an
    // immutable attribute, a read-only mount, SELinux. Proving it by writing is
    // cheaper than enumerating the reasons, and it happens once at startup.
    let probe = dir.join(".atlasctl-write-probe");
    std::fs::write(&probe, b"")
        .with_context(|| format!("{} is not writable by this user", dir.display()))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(unix)]
fn facts(dir: &Path) -> Result<DirFacts> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::metadata(dir).with_context(|| format!("inspecting {}", dir.display()))?;
    Ok(DirFacts {
        owner_uid: md.uid(),
        mode: md.mode(),
        current_uid: rustix::process::getuid().as_raw(),
    })
}

#[cfg(not(unix))]
fn facts(_dir: &Path) -> Result<DirFacts> {
    // No uid model to reason about; the write probe in `ensure_usable` is the
    // whole check there.
    Ok(DirFacts {
        owner_uid: 0,
        mode: 0o700,
        current_uid: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        PathBuf::from("/home/someone/.config/atlasctl")
    }

    /// The reported failure: a directory left behind by another user. The old
    /// message was `Permission denied (os error 13)` and nothing else.
    #[test]
    fn a_directory_owned_by_another_user_names_both_remedies() {
        let why = advice(
            &dir(),
            DirFacts {
                owner_uid: 0,
                mode: 0o700,
                current_uid: 1000,
            },
        )
        .expect("must refuse");
        assert!(why.contains("owned by uid 0"), "{why}");
        assert!(why.contains("this process is uid 1000"), "{why}");
        assert!(why.contains("chown"), "{why}");
        assert!(why.contains("--config-dir"), "{why}");
    }

    /// Moving the directory with XDG_CONFIG_HOME takes agent.key with it, so
    /// the node comes back with a new identity and no pins. Anyone reading this
    /// error is one command away from doing exactly that.
    #[test]
    fn the_advice_warns_against_the_identity_losing_workaround() {
        let why = advice(
            &dir(),
            DirFacts {
                owner_uid: 0,
                mode: 0o700,
                current_uid: 1000,
            },
        )
        .expect("must refuse");
        assert!(why.contains("XDG_CONFIG_HOME"), "{why}");
        assert!(why.contains("agent.key"), "{why}");
    }

    #[test]
    fn our_own_directory_with_the_write_bit_off_says_chmod_not_chown() {
        let why = advice(
            &dir(),
            DirFacts {
                owner_uid: 1000,
                mode: 0o500,
                current_uid: 1000,
            },
        )
        .expect("must refuse");
        assert!(why.contains("chmod u+rwx"), "{why}");
        assert!(!why.contains("chown"), "must not blame ownership: {why}");
    }

    #[test]
    fn a_normal_directory_is_not_complained_about() {
        assert!(
            advice(
                &dir(),
                DirFacts {
                    owner_uid: 1000,
                    mode: 0o700,
                    current_uid: 1000,
                }
            )
            .is_none()
        );
    }
}
