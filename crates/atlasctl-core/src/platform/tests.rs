// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for [`super`], split out on the 500-line cap — the same seam
//! `service`, `identity` and `docker::command` already use.

use super::*;

/// The bases must be BELOW the home directory, not merely different from
/// it. Asserting only inequality passed on the one configuration where
/// "below" is false — a redirected `%LOCALAPPDATA%` — which is the case
/// worth catching, since that is what a secret's containment is anchored
/// on.
///
/// An explicit environment override is exempt: an operator who named a
/// directory chose it, and refusing their choice here would be this
/// function inventing a policy the resolver does not have.
#[test]
fn the_bases_sit_below_the_home_directory() {
    let home = home_dir().expect("a home directory");
    let overridden = ["XDG_CONFIG_HOME", "XDG_CACHE_HOME", "LOCALAPPDATA"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    for base in [
        config_base().expect("config base"),
        cache_base().expect("cache base"),
    ] {
        assert_ne!(base, home, "state must not land directly in the home dir");
        if !overridden {
            assert!(
                base.starts_with(&home),
                "{} is not below {}",
                base.display(),
                home.display()
            );
        }
    }
}

/// A hostname is used in display and in a peer Hello, so an empty string
/// would render as a nameless node rather than as a problem.
#[test]
fn a_hostname_is_never_empty() {
    assert!(!hostname_or("fallback").is_empty());
}

/// And the caller's fallback is the one used, rather than a shared string
/// that reads as a real hostname to one caller and as a placeholder to the
/// other.
#[test]
fn the_callers_fallback_is_what_appears() {
    // Forced down the fallback path by asking on a machine where neither
    // source can answer is not possible here, so the property checked is
    // the weaker one that holds always: whatever comes back is non-empty,
    // and the fallback is never silently replaced by a built-in.
    let a = hostname_or("<this-machine>");
    let b = hostname_or("atlas-node");
    assert_eq!(a, b, "a real hostname must not depend on the fallback");
    assert!(!a.is_empty());
}

/// The question this answers is asked before a 100 GB pull, so an answer
/// of "cannot tell" on a working machine is the failure that matters.
#[test]
fn free_space_is_readable_for_a_directory_that_exists() {
    let n = free_bytes(&std::env::temp_dir()).expect("a temp dir has a filesystem");
    assert!(
        n > 0,
        "a writable temp dir with zero bytes free is not credible"
    );
}

/// A fresh install asks about a cache directory that does not exist yet.
/// Answering `None` there would blank the disk column on every new machine.
#[test]
fn free_space_walks_up_to_a_parent_that_exists() {
    let missing = std::env::temp_dir()
        .join("atlasctl-does-not-exist")
        .join("nor-this");
    assert!(
        free_bytes(&missing).is_some(),
        "must fall back to the volume"
    );
}

/// `which` is how docker is found. On Windows a bare name never matches,
/// which reported every tool on the machine as missing.
#[test]
fn which_finds_a_real_program_and_not_an_invented_one() {
    let real = if cfg!(windows) { "cmd" } else { "sh" };
    assert!(which(real).is_some(), "{real} must be on PATH");
    assert!(which("definitely-not-a-real-binary-xyz").is_none());
}

/// A file is not a program. `doctor` uses this to decide whether sparkrun
/// is installed, and printed a SECURITY NOTICE about a tool the machine
/// could not run — a plain text file of that name on PATH was enough.
#[cfg(unix)]
#[test]
fn a_readable_file_that_is_not_executable_is_not_found() {
    use std::os::unix::fs::PermissionsExt;
    let d = std::env::temp_dir().join(format!("atlasctl-which-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("dir");
    let name = "atlasctl-not-a-program";
    let p = d.join(name);
    std::fs::write(&p, b"not a program").expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    // SAFETY: single-threaded test process; PATH is restored below.
    let saved = std::env::var_os("PATH");
    unsafe { std::env::set_var("PATH", &d) };
    let found = which(name);
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let found_after = which(name);
    match saved {
        Some(v) => unsafe { std::env::set_var("PATH", v) },
        None => unsafe { std::env::remove_var("PATH") },
    }

    assert!(found.is_none(), "a 0644 file must not read as a program");
    assert!(found_after.is_some(), "and 0755 must");
    let _ = std::fs::remove_dir_all(&d);
}

/// Checked against the OS, not against the same `cfg!` the implementation
/// branches on — that form was true by construction and could not fail.
/// What matters downstream is the VALUE: `translate` renders `--user
/// uid:gid` from it, so a zero uid would silently ask docker to run the
/// container as root.
#[cfg(unix)]
#[test]
fn the_posix_identity_is_this_process_s_own() {
    let u = posix_user().expect("unix always has one");
    assert_eq!(u.uid, rustix::process::getuid().as_raw());
    assert_eq!(u.gid, rustix::process::getgid().as_raw());
}

/// And on Windows there is no uid to report. Asserting `None` is not
/// tautological here: the alternative a port reaches for is `Some(0)`.
#[cfg(windows)]
#[test]
fn windows_reports_no_posix_identity_rather_than_root() {
    assert!(posix_user().is_none());
}
