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
    .map_err(|e| join_failure(&e))?;

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

/// Turn a failed join into words the operator can act on.
///
/// Extracted so the WORDING is testable. The defect this replaced was entirely
/// a wording defect — the code was blamed for a failure that never presented
/// it — and a fix to wording that no test can see is one refactor away from
/// coming back.
fn join_failure(e: &anyhow::Error) -> anyhow::Error {
    // Blaming the code when nothing ever answered sends the operator to redo
    // the one step that was not the problem. A refused or timed-out dial means
    // this machine never presented the code at all, so it is still good — what
    // is wrong is on the OTHER machine, and that is where the next command has
    // to be run.
    if atlasctl_agent::peer::reach::was_never_reached(e) {
        anyhow::anyhow!(
            concat!(
                "could not reach that machine — nothing answered on its peer port.\n",
                "  {e:#}\n",
                "The code was never presented, so it is still good: do not mint a new one.\n",
                "On THAT machine, check the `peers:` line of:\n",
                "    atlasctl doctor\n",
                "A peer channel that is not listening is the usual cause. It retries, so ",
                "something else may still be holding the port.",
            ),
            e = e
        )
    } else {
        anyhow::anyhow!(
            concat!(
                "could not join the fleet. It answered, so the code is the likely ",
                "problem — it expires, and is good for one machine only. Mint a fresh ",
                "one if this is not the first try.\n",
                "  {e:#}",
            ),
            e = e
        )
    }
}

#[cfg(test)]
mod join_failure_tests {
    use super::join_failure;

    fn unreachable() -> anyhow::Error {
        anyhow::Error::new(atlasctl_agent::peer::reach::NeverReached)
            .context("192.168.68.67:34334: Connection refused (os error 111)")
    }

    /// Nothing answered: the code is exonerated, and the next command is aimed
    /// at the machine that is actually broken.
    #[test]
    fn an_unreachable_target_does_not_blame_the_code() {
        let msg = format!("{:#}", join_failure(&unreachable()));
        assert!(msg.contains("still good"), "must exonerate the code: {msg}");
        assert!(
            !msg.contains("Mint a fresh"),
            "must NOT send them to mint a new code — that is the dead end: {msg}"
        );
        assert!(
            msg.contains("doctor"),
            "must aim the next command at the other machine: {msg}"
        );
        assert!(
            msg.contains("Connection refused"),
            "must keep the underlying cause: {msg}"
        );
    }

    /// Answered and refused: the code IS the likely problem, and saying so is
    /// correct here. The two branches must not converge on one vague message.
    #[test]
    fn a_refusal_by_the_far_end_still_points_at_the_code() {
        let e = anyhow::anyhow!("that code has already been used");
        let msg = format!("{:#}", join_failure(&e));
        assert!(msg.contains("Mint a fresh"), "{msg}");
        assert!(
            !msg.contains("still good"),
            "a spent code must not be called good: {msg}"
        );
    }

    /// No run of spaces inside either message.
    ///
    /// Written as ordinary string continuations, rustfmt reflows these into the
    /// message body. That happened to the first draft of this change and to a
    /// test fixture the same day, and it is invisible in review.
    #[test]
    fn neither_message_carries_reflowed_whitespace() {
        for msg in [
            format!("{:#}", join_failure(&unreachable())),
            format!("{:#}", join_failure(&anyhow::anyhow!("refused"))),
        ] {
            assert!(
                !msg.contains("  ") || msg.contains("\n  "),
                "reflowed whitespace leaked into the message: {msg:?}"
            );
        }
    }
}
