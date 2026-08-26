// SPDX-License-Identifier: AGPL-3.0-only

//! Installing and removing the agent's background service.
//!
//! Split from [`super::agent`] for size. The decisions live in
//! [`crate::service`], which is pure and tested; these two functions are the
//! composition root — they discover this machine's paths and identity, hand
//! the real filesystem and process runner in, and describe what happened.

use anyhow::{Context, Result};

/// Install the agent as a background service for this user.
///
/// # Errors
/// If the platform has no supported supervisor, or a required step fails.
pub fn install(args: &crate::cli::AgentInstallArgs) -> Result<()> {
    let exe = std::env::current_exe().context("could not find this binary's own path")?;
    let home = crate::hostinfo::home_dir()?;
    let invocation = crate::service::plan::AgentInvocation {
        exe,
        port: args.port,
        client: args.client,
        discovery: !args.no_discovery,
        browser: !args.no_browser,
    };
    let done = crate::service::install(
        &atlasctl_core::io::StdFileSystem,
        &atlasctl_core::io::StdProcessRunner,
        &invocation,
        &home,
        uid_of(&home),
    )?;

    println!("agent installed and started");
    println!("  unit: {}", done.unit_path.display());
    println!("  port: {}", args.port);
    if args.client {
        println!("  mode: control only — it will not run a model here");
    }
    for s in &done.skipped {
        // Surfaced, never swallowed: on a headless box a failed enable-linger
        // is the difference between an agent that survives logout and one that
        // does not, and only the operator can decide whether that matters.
        println!("  note: {s}");
    }
    if done.skipped.iter().any(|s| s.contains("enable-linger")) {
        println!(
            "        without lingering the agent stops when you log out. Run\n\
             \x20       `sudo loginctl enable-linger $USER` to keep it up on a headless machine."
        );
    }
    println!("\nPair your browser with: atlasctl agent token");
    Ok(())
}

/// Remove the background service.
///
/// # Errors
/// If the platform has no supported supervisor, or the unit cannot be removed.
pub fn uninstall() -> Result<()> {
    let home = crate::hostinfo::home_dir()?;
    let path = crate::service::uninstall(
        &atlasctl_core::io::StdFileSystem,
        &atlasctl_core::io::StdProcessRunner,
        &home,
        uid_of(&home),
    )?;
    println!("agent service removed ({})", path.display());
    println!("the binary and your pairings are untouched; `atlasctl agent run` still works");
    Ok(())
}

/// This user's id, which launchd needs in order to name a session.
///
/// Taken from the owner of the home directory rather than from `getuid`, so
/// this needs no `libc` dependency — worth avoiding in a project that exists
/// because of a supply-chain compromise, for a number used on exactly one
/// platform. A home directory is owned by the user whose home it is.
///
/// Zero when it cannot be determined: only launchd reads it, and on macOS a
/// home directory always has an owner, so the fallback is unreachable there.
#[cfg(unix)]
fn uid_of(home: &std::path::Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home).map(|m| m.uid()).unwrap_or(0)
}

#[cfg(not(unix))]
fn uid_of(_home: &std::path::Path) -> u32 {
    0
}
