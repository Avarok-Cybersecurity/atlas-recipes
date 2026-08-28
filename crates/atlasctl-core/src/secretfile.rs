// SPDX-License-Identifier: AGPL-3.0-only

//! Writing and checking the files nobody else may read.
//!
//! Three files qualify: `browser.token` pairs a browser, `agent.key` is this
//! node's private identity, and anything derived from them. They had two
//! separate implementations of "write this privately" — one in `token`, one in
//! `identity` — which is one implementation too many for a rule that has to
//! hold everywhere.
//!
//! # What "private" means per platform
//!
//! On unix it is a mode: `0600` at creation, verified on read. The mode is set
//! *at* creation rather than chmod-ed after, so the secret never exists at the
//! ambient umask, however briefly.
//!
//! On Windows there is no mode, and inventing one would be theatre. Protection
//! comes from the directory: `%LOCALAPPDATA%` carries an inherited ACL granting
//! the owning user, SYSTEM and Administrators — the same trust boundary as
//! `~/.config` at `0700`, where root reads everything too. A new file inherits
//! that ACL, so writing normally is writing privately.
//!
//! What that does *not* cover is a secret written somewhere else, which is
//! reachable: `--config-dir` takes any path the operator names, and a network
//! share or `C:\ProgramData` is world-readable. So the Windows check is
//! containment — the secret must live under this user's profile. It is a
//! weaker statement than the unix mode check and is deliberately not dressed
//! up as an equivalent one; a DACL walk that would also catch a hand-widened
//! ACL on the profile itself is left for when there is a reason to believe
//! that happens.

use anyhow::{Context, Result, bail};
use std::path::Path;

/// Create or replace a file that only its owner may read.
///
/// # Errors
/// If the file cannot be created or written.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    #[cfg(windows)]
    {
        // Refused before the bytes exist, not after: a secret written to a
        // share and then reported is a secret that was on the share.
        verify_location(path)?;
        std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Refuse a secret this machine's other accounts can read.
///
/// # Errors
/// With the remedy, when the file is not private.
pub fn verify(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .with_context(|| format!("inspecting {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} is readable by other accounts (mode {:o}). Another user on \
                 this machine may already have it; rotate it with \
                 `atlasctl agent token --rotate`.",
                path.display(),
                mode & 0o777
            );
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        verify_location(path)
    }
}

/// The Windows half: a secret must live inside this user's own profile, where
/// the inherited ACL is the protection.
#[cfg(windows)]
fn verify_location(path: &Path) -> Result<()> {
    let Ok(profile) = std::env::var("USERPROFILE") else {
        // Not a silent pass. Without a profile there is no directory whose ACL
        // can be relied on, and saying so beats writing a secret anyway.
        bail!(
            "USERPROFILE is not set, so there is no directory whose permissions \
             protect {}. Set it, or run atlasctl as a normal desktop user.",
            path.display()
        );
    };
    let profile = std::path::Path::new(&profile);
    // Compared after canonicalising the PARENT: the file itself may not exist
    // yet, and a path containing `..` would otherwise pass a prefix test while
    // resolving somewhere else entirely.
    let parent = path.parent().unwrap_or(path);
    let resolved =
        std::fs::canonicalize(parent).with_context(|| format!("resolving {}", parent.display()))?;
    let profile = std::fs::canonicalize(profile)
        .with_context(|| format!("resolving USERPROFILE {}", profile.display()))?;
    if !resolved.starts_with(&profile) {
        bail!(
            "{} is outside your user profile ({}), so Windows does not restrict \
             who can read it. Keep this node's secrets under \
             %LOCALAPPDATA%\\atlasctl, or point --config-dir at a directory \
             inside your profile.",
            resolved.display(),
            profile.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("atlasctl-secret-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        d
    }

    /// The property both platforms owe: what `write` produces, `verify`
    /// accepts. A port that satisfies one and not the other locks the operator
    /// out of a file this very program just wrote.
    #[test]
    fn what_write_produces_verify_accepts() {
        let d = tmp("roundtrip");
        let p = d.join("browser.token");
        write(&p, b"hunter2").expect("writes");
        verify(&p).expect("must accept its own output");
        assert_eq!(std::fs::read(&p).unwrap(), b"hunter2");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Replacing must not widen. `OpenOptions` reuses an existing file's mode,
    /// so a secret rotated into a file someone had chmod-ed stays readable
    /// unless rotation is checked too.
    #[cfg(unix)]
    #[test]
    fn rewriting_over_a_widened_file_is_caught() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("rotate");
        let p = d.join("agent.key");
        write(&p, b"first").expect("writes");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write(&p, b"second").expect("writes");
        let err = verify(&p).expect_err("a widened secret must be refused");
        assert!(
            err.to_string().contains("readable by other accounts"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The unix guarantee, stated as the bits: created private, with no window
    /// at the ambient umask.
    #[cfg(unix)]
    #[test]
    fn a_new_secret_is_created_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("mode");
        let p = d.join("agent.key");
        write(&p, b"k").expect("writes");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The Windows guarantee: a secret outside the profile is refused BEFORE it
    /// is written, so the refusal is not a report of a file that already leaked.
    #[cfg(windows)]
    #[test]
    fn a_secret_outside_the_profile_is_refused_before_it_exists() {
        let d = std::path::Path::new("C:\\Windows\\Temp")
            .join(format!("atlasctl-outside-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("temp dir");
        let p = d.join("agent.key");
        let err = write(&p, b"k").expect_err("outside the profile must be refused");
        assert!(
            err.to_string().contains("outside your user profile"),
            "{err}"
        );
        assert!(!p.exists(), "the secret must not have been written");
        let _ = std::fs::remove_dir_all(&d);
    }
}
