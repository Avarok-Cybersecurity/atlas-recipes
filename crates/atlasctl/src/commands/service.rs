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
    // Parsed before anything is installed. A malformed invitation should cost
    // the operator a retyped argument, not a service they now have to remove.
    let join = args
        .join
        .as_deref()
        .map(crate::joinarg::parse)
        .transpose()?;

    let exe = std::env::current_exe().context("could not find this binary's own path")?;
    let home = crate::hostinfo::home_dir()?;
    let invocation = crate::service::plan::AgentInvocation {
        exe,
        port: args.port,
        client: args.client,
        discovery: !args.no_discovery,
        browser: !args.no_browser,
        // Only when the operator chose one. Recording the resolved default
        // would pin today's default into a unit that outlives it, which is the
        // same trap the port comment warns about — except this one would move
        // the node's identity rather than its port.
        config_dir: std::env::var_os(crate::configdir::DIR_ENV).map(std::path::PathBuf::from),
    };
    let done = crate::service::install(
        &atlasctl_core::io::StdFileSystem,
        &atlasctl_core::io::StdProcessRunner,
        &invocation,
        &home,
        uid_of(&home),
    )?;

    if done.running {
        println!("agent installed and started");
    } else {
        // Installed is true; started is not. Saying "started" here is how an
        // operator ends up pairing a browser against a five-second crash loop.
        println!("agent installed, but it is NOT running");
        println!("  the unit is enabled and the supervisor accepted it, so this is");
        println!("  the agent exiting at startup. See why with:");
        println!("    journalctl --user -u atlasctl-agent -n 50");
    }
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
    if let Some(join) = join {
        join_fleet(&join, args.grant_control)?;
    } else {
        println!("\nPair your browser with: atlasctl agent token");
    }
    Ok(())
}

/// Dial the machine that invited us and complete the ceremony.
///
/// Runs as the initiator against a code the *other* side minted, which is the
/// inverse of `atlasctl peer add`. Everything about what a pairing means is
/// still [`atlasctl_agent::peer::join::dial_and_pair`]; only the direction
/// differs.
fn join_fleet(join: &crate::joinarg::Join, grant_control: bool) -> Result<()> {
    use atlasctl_agent::identity::{Identity, PinStore};

    let dir = crate::hostinfo::config_dir()?;
    crate::configdir::ensure_usable(&dir)?;
    let identity = Identity::load_or_create(&dir)?;
    let pins = PinStore::new(&dir);

    let addrs = atlasctl_agent::discovery::resolve_manual(
        &join.host,
        atlasctl_agent::peer::DEFAULT_PEER_PORT,
    )?;
    let addr = *addrs
        .first()
        .ok_or_else(|| anyhow::anyhow!("{} resolved to no addresses", join.host))?;

    println!("\njoining the fleet at {addr}…");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;
    let paired = rt
        .block_on(atlasctl_agent::peer::join::dial_and_pair(
            &identity,
            pins.clone(),
            addr,
            &join.code,
        ))
        .with_context(|| {
            format!(
                "could not join {addr}. The code expires, and is good for one machine only — mint a fresh one if this is not the first try."
            )
        })?;

    atlasctl_agent::fleet::record_pairing(
        &pins,
        paired.node,
        &paired.public_key,
        atlasctl_protocol::fleet::DisplayName::new(&paired.name),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        Some(addr.ip().to_string()),
        // Pairing authenticates; the grant is a separate decision, made HERE
        // because it is this machine's authority being given away. It arrives
        // as `--grant-control` on the line the operator is pasting at this
        // keyboard, so it is explicit and readable before it runs — never
        // implied by joining, and never decided by the machine that invited us.
        grant_control,
    )?;

    println!("joined {} ({})", paired.name, paired.node.short());
    println!("  verification words: {}", paired.verification);
    println!("  the browser that invited this machine is showing the same words.");
    println!(
        "  If it is showing something else, run `atlasctl peer remove {}`.",
        paired.node.short()
    );
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
