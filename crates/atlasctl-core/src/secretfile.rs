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
    // Written to a sibling temp file and RENAMED over the target, never
    // truncate-then-write. `peers.json` goes through here, and truncate-first
    // means a crash, a full disk or a kill between the two leaves a torn file --
    // after which every `load()` fails to parse and the agent fails closed on
    // EVERY peer at once. Rename is atomic on both platforms (Windows replaces
    // an existing target), so a reader sees the old file or the new one.
    //
    // ⚠ This makes each write indivisible; it does NOT serialise two writers.
    // The daemon and a separate `atlasctl peer remove` still read-modify-write
    // the whole file, so an interleaving can lose one side's change -- including
    // un-revoking a peer that was just removed. Closing that needs a lock file
    // (or routing CLI mutations through the running daemon) and is a bigger
    // change than this one.
    // Resolve a symlink to its target FIRST. The previous truncate-then-write
    // opened `path` and wrote THROUGH a symlink, so a stowed/dotfiles setup that
    // links `agent.key` or `peers.json` elsewhere kept working. A rename would
    // replace the link itself with a regular file -- silently destroying that
    // arrangement, and, worse for a rotation, leaving the OLD secret live at the
    // real target, which is the one thing rotating exists to prevent.
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    let dir = path.parent().unwrap_or(Path::new("."));
    // Unique PER CALL, not per process. A pid-only suffix is not unique inside
    // the daemon: `peers.json` is written from the address-poll loop, from a
    // browser session's unpair, and from a join window's pin write, all on the
    // same multi-threaded runtime. Two of those sharing one temp name is the
    // torn file this function exists to prevent, arrived at from inside a single
    // process -- and the loser's cleanup could delete the winner's temp before
    // its rename. The counter makes each attempt its own file.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{}.tmp{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("secret"),
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        // Durable before it is visible: a rename that beats the data to disk
        // would survive a power cut as a correctly-named empty file.
        f.sync_all()
            .with_context(|| format!("flushing {}", tmp.display()))?;

        // Carry the EXISTING file's mode over, rather than always landing 0600.
        // `write` and `verify` are split on purpose in this module: write does
        // not police permissions, verify detects them. A fresh 0600 on every
        // write would silently re-privatise a secret someone had widened, and
        // erase the only evidence that other accounts could read it -- which is
        // exactly what `rewriting_over_a_widened_file_is_caught` pins.
        if let Ok(meta) = std::fs::metadata(path) {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode() & 0o7777;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
                .with_context(|| format!("carrying the mode of {}", path.display()))?;
        }
    }
    #[cfg(windows)]
    {
        // Refused before the bytes exist, not after: a secret written to a
        // share and then reported is a secret that was on the share. The temp
        // file is a sibling, so checking the destination covers both.
        verify_location(path)?;
        std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    }

    // The DIRECTORY entry too, not just the bytes. Without this the rename can
    // be undone by a power cut and a write that reported success -- a rotated
    // token, a revoked pin -- silently rolls back to the previous contents.
    // Best-effort: not every filesystem lets you open a directory for sync, and
    // failing the write over an unsyncable parent would be worse than the risk.
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Do not leave the temp file behind to be mistaken for state, or to
        // accumulate one per crashed process.
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", path.display()));
    }
    #[cfg(unix)]
    {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
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

    /// Two writers in ONE process must not share a temp file. Inside the daemon
    /// `peers.json` is written from the address-poll loop, a browser unpair and a
    /// join window's pin write, on the same multi-threaded runtime; with a
    /// pid-only temp name they truncate each other and one renames the mixed
    /// bytes into place -- the torn file this function exists to prevent,
    /// reachable without a second process at all.
    #[test]
    fn concurrent_writers_in_one_process_do_not_share_a_temp_file() {
        let d = tmp("concurrent");
        let p = d.join("peers.json");
        let a = format!("{{\"a\":\"{}\"}}", "a".repeat(60_000));
        let b = format!("{{\"b\":\"{}\"}}", "b".repeat(90_000));
        let target = &p;
        std::thread::scope(|s| {
            for body in [a.as_bytes(), b.as_bytes()] {
                s.spawn(move || {
                    for _ in 0..25 {
                        write(target, body).expect("writes");
                    }
                });
            }
        });
        // Whoever landed last, the file must be EXACTLY one of the two -- never
        // a prefix, a mixture, or a length in between.
        let got = std::fs::read(&p).expect("reads");
        assert!(
            got == a.as_bytes() || got == b.as_bytes(),
            "torn file: {} bytes, expected {} or {}",
            got.len(),
            a.len(),
            b.len()
        );
        let strays: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(strays.is_empty(), "left temp files behind: {strays:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The reason this write goes through a temp file: `peers.json` is
    /// rewritten whole on every pin change, and a truncate-then-write left a
    /// torn file if anything interrupted it -- after which every load fails to
    /// parse and the agent fails closed on EVERY peer at once. A reader must see
    /// the old bytes or the new ones, never a prefix.
    #[test]
    fn a_replaced_secret_is_never_seen_half_written() {
        let d = tmp("atomic");
        let p = d.join("peers.json");
        write(&p, b"{\"peers\":[\"first\"]}").expect("writes");
        // A much larger second write: a truncating writer would leave the file
        // observably shorter than either version at some point.
        let big = format!("{{\"peers\":[\"{}\"]}}", "x".repeat(200_000));
        write(&p, big.as_bytes()).expect("writes");
        let got = std::fs::read(&p).expect("reads");
        assert_eq!(
            got.len(),
            big.len(),
            "the file is exactly one of the versions"
        );

        // And no temp file is left lying around to be mistaken for state.
        let strays: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(strays.is_empty(), "left temp files behind: {strays:?}");
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
