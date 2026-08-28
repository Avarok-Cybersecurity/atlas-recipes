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
    // Anchored on the directory this program WRITES under, not on the profile.
    // Those differ when `%LOCALAPPDATA%` is redirected — a supported Windows
    // configuration — and anchoring on the profile made atlasctl refuse its own
    // default config directory. It failed the other way too: a roaming
    // `%USERPROFILE%` on a UNC share would have PASSED containment while
    // sitting on exactly the network share this check exists to refuse.
    let anchor = crate::platform::config_base()
        .context("there is no directory whose permissions could protect a secret")?;
    if is_unc(&anchor) {
        bail!(
            "{} is a network path, so Windows does not restrict who can read \
             secrets kept there. Point %LOCALAPPDATA% or --config-dir at a \
             local directory.",
            anchor.display()
        );
    }

    // Canonicalise the FILE when it exists, falling back to its parent only
    // when it does not. Resolving the parent alone left the last component
    // unchecked, so `…\atlasctl\agent.key` could be a symlink to
    // `C:\ProgramData\…`: parent inside the profile, bytes outside it, and
    // `verify` passing forever after.
    let (resolved, described) = match std::fs::canonicalize(path) {
        Ok(p) => (p, path.to_path_buf()),
        Err(_) => {
            let parent = path.parent().unwrap_or(path);
            (
                std::fs::canonicalize(parent)
                    .with_context(|| format!("resolving {}", parent.display()))?,
                parent.to_path_buf(),
            )
        }
    };
    let anchor = std::fs::canonicalize(&anchor)
        .with_context(|| format!("resolving {}", anchor.display()))?;
    if !resolved.starts_with(&anchor) {
        bail!(
            "{} resolves to {}, outside {} — so Windows does not restrict who \
             can read it. Keep this node's secrets under %LOCALAPPDATA%\\atlasctl, \
             or point --config-dir at a directory inside it.",
            described.display(),
            plain(&resolved),
            plain(&anchor)
        );
    }
    Ok(())
}

/// A canonicalised path as an operator would write it.
///
/// `canonicalize` returns the `\\?\` extended-length form, which is correct and
/// unreadable. Stripped for DISPLAY only — the comparison still uses the
/// canonical values, because resolving them is the whole point.
#[cfg(windows)]
fn plain(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| s.strip_prefix(r"\\?\").map(ToOwned::to_owned))
        .unwrap_or(s)
}

/// Whether a path names a network share: `\\server\share`, or the `\\?\UNC\…`
/// form that canonicalize produces for one.
#[cfg(windows)]
fn is_unc(p: &Path) -> bool {
    let s = p.to_string_lossy();
    s.starts_with(r"\\?\UNC\") || (s.starts_with(r"\\") && !s.starts_with(r"\\?\"))
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
        let err = write(&p, b"k").expect_err("outside the anchor must be refused");
        let msg = err.to_string();
        // The PROPERTY, not the phrasing: it must name what was refused and the
        // remedy, and it must not leak the extended-length prefix that
        // canonicalize returns into a message an operator reads.
        assert!(msg.contains("atlasctl-outside"), "{msg}");
        assert!(msg.contains("--config-dir"), "{msg}");
        assert!(
            !msg.contains(r"\\?\"),
            "the extended-length prefix leaked into an operator message: {msg}"
        );
        assert!(!p.exists(), "the secret must not have been written");
        let _ = std::fs::remove_dir_all(&d);
    }
}
