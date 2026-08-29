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
        log_file: None,
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
        println!("  the agent exiting at startup.");
        // Ask, do not assert. This used to state "the usual cause is the port"
        // unconditionally -- and a real install then printed the ACTUAL cause a
        // few lines later (a config directory owned by another uid). Two
        // confident, contradictory explanations is worse than one honest "I
        // cannot see why": the operator acts on the first, nothing changes, and
        // they stop trusting the second.
        match startup_obstacle(args.port) {
            Some(why) => {
                for line in why.lines() {
                    println!("  {line}");
                }
            }
            None => {
                println!("  This installer could not see why from here — the log can:");
            }
        }
        println!("  See the log with:");
        match done.kind {
            // `journalctl` does not exist on macOS, and it was the only thing
            // offered here — so the one diagnostic an operator was handed
            // failed too, on the platform where this path is most likely.
            crate::service::plan::ServiceKind::Launchd => {
                println!("    tail -n 50 ~/Library/Logs/atlasctl-agent.log");
            }
            crate::service::plan::ServiceKind::Systemd => {
                println!("    journalctl --user -u atlasctl-agent -n 50");
            }
            // Task Scheduler captures nothing, which is why the agent is
            // started with `--log-file`; this is that file. Quoted, because a
            // profile path with a space in it is the common case and an
            // unquoted one fails when pasted.
            crate::service::plan::ServiceKind::ScheduledTask => {
                println!(
                    "    Get-Content -Tail 50 '{}'",
                    crate::service::plan::windows_log_path(&home).display()
                );
            }
        }
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

    // Every alternative the inviter offered, flattened in the order given. A
    // host that will not resolve is recorded rather than fatal: the LAST entry
    // is often the only one this machine's network can even name.
    let mut addrs: Vec<std::net::SocketAddr> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for host in &join.hosts {
        match atlasctl_agent::discovery::resolve_manual(
            host,
            atlasctl_agent::peer::DEFAULT_PEER_PORT,
        ) {
            Ok(found) => addrs.extend(found),
            Err(e) => unresolved.push(format!("{host}: {e:#}")),
        }
    }
    if addrs.is_empty() {
        anyhow::bail!(
            "none of the addresses in --join could be resolved — {}",
            unresolved.join("; ")
        );
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;

    // Same walk, same stop rule as pairing a discovered machine: `peer::reach`
    // owns both, so the joining direction cannot drift from the other one.
    let (addr, paired) = atlasctl_agent::peer::reach::walk(&addrs, |addr| {
        println!("\njoining the fleet at {addr}…");
        rt.block_on(atlasctl_agent::peer::join::dial_and_pair(
            &identity,
            pins.clone(),
            addr,
            &join.code,
        ))
    })
    .map_err(|e| {
        anyhow::anyhow!(
            "could not join the fleet. The code expires, and is good for one machine only — mint a fresh one if this is not the first try.\n  {e:#}"
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

    // Sanitised: the name is the peer's, and the peer is not trusted yet.
    println!(
        "joined {} ({})",
        atlasctl_protocol::fleet::DisplayName::new(&paired.name).as_str(),
        paired.node.short()
    );
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
pub(crate) fn uid_of(home: &std::path::Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(home).map(|m| m.uid()).unwrap_or(0)
}

#[cfg(not(unix))]
pub(crate) fn uid_of(_home: &std::path::Path) -> u32 {
    0
}

/// The reason an agent would exit at startup, when it is one this process can
/// actually check.
///
/// Both checks are observations, not guesses: the config directory is inspected,
/// and the port is probed by connecting to it. `None` means neither is wrong,
/// which is a useful answer — it tells the operator to go and read the log
/// rather than chase a cause that was never there.
fn startup_obstacle(port: u16) -> Option<String> {
    if let Ok(dir) = crate::configdir::resolve()
        && let Some(why) = crate::configdir::diagnose(&dir)
    {
        return Some(format!(
            "The reason is this node's config directory:\n{why}"
        ));
    }
    if port_is_taken(port) {
        return Some(format!(
            "Something is already listening on {port}, so the agent cannot bind it.\n             The usual cause is an agent started by hand in another terminal.\n             Stop it, or install on a different port with `--port`."
        ));
    }
    None
}

/// Whether something already answers on the loopback port.
///
/// A connect, not a bind: binding to test would race the agent the supervisor is
/// starting right now and could report the port taken by the very process being
/// installed.
fn port_is_taken(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
}

#[cfg(test)]
mod obstacle_tests {
    use super::{port_is_taken, startup_obstacle};

    /// The probe must answer about the port it was ASKED about.
    ///
    /// This is the check that replaced an asserted "the usual cause is the
    /// port". An assertion that is right by luck is no better than the one it
    /// replaced, so both directions are pinned.
    #[test]
    fn the_port_probe_distinguishes_held_from_free() {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let held_port = held.local_addr().expect("addr").port();
        assert!(port_is_taken(held_port), "a bound port must read as taken");

        // Bind then drop: a port we know is free, rather than a number we hope
        // nothing on the machine is using.
        let free_port = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
            l.local_addr().expect("addr").port()
        };
        assert!(
            !port_is_taken(free_port),
            "a closed port must not read as taken"
        );
    }

    /// With a free port and a usable config directory, there is NO obstacle —
    /// and saying so is the point.
    ///
    /// The bug this replaced was a confident wrong answer. "I cannot see why"
    /// is the honest one when neither thing this can check is wrong, and it is
    /// what sends the operator to the log instead of to a dead end.
    #[test]
    fn nothing_wrong_reports_nothing_rather_than_guessing() {
        let free_port = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind");
            l.local_addr().expect("addr").port()
        };
        // The config-dir half depends on this machine's real directory, so this
        // asserts only what is safe to assert: whatever it answers, it must not
        // blame a port that nothing is holding.
        if let Some(why) = startup_obstacle(free_port) {
            assert!(
                !why.contains("already listening"),
                "must not blame a free port: {why}"
            );
        }
    }
}
