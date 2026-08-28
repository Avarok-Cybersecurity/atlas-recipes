// SPDX-License-Identifier: AGPL-3.0-only

//! `agent token` and `agent status` — the two verbs that ask an agent about
//! itself rather than running one.
//!
//! Split from `commands/agent.rs` on the 500-line cap. The seam is real: `run`
//! builds and owns a process, and these two only read state a running agent
//! already published.

use anyhow::{Result, bail};

use crate::cli::{AgentStatusArgs, AgentTokenArgs};
use crate::hostinfo;
use atlasctl_agent::token;

/// Print, or rotate, the pairing token.
pub fn token(args: &AgentTokenArgs) -> Result<()> {
    let dir = hostinfo::usable_config_dir()?;
    let tok = if args.rotate {
        let t = token::rotate(&dir)?;
        eprintln!("token rotated — any browser already paired must be given the new one");
        t
    } else {
        token::load_or_create(&dir)?
    };
    println!("{tok}");
    Ok(())
}

/// What to suggest when nothing answered on `port`.
///
/// Pure so it can be tested: the probe itself is a socket connect, and the
/// interesting behaviour is which advice an operator gets, not whether the
/// kernel can refuse a connection. `installed` is passed in for the same
/// reason — whether a unit exists on disk is I/O, and which advice follows
/// from it is a rule.
#[must_use]
pub fn status_advice(port: u16, installed: bool) -> Vec<String> {
    let mut out = Vec::new();
    if port == atlasctl_agent::DEFAULT_PORT {
        // Naming the flag matters more than it looks: an operator who installed
        // on another port has no reason to suspect this command only ever
        // asked one, and the old advice ("start it with agent run") told them
        // to launch a second agent that would then fail to bind.
        out.push(
            "if you installed it on another port, check that one:\n  \
             atlasctl agent status --port <PORT>"
                .to_owned(),
        );
    }
    // `agent install`, not `agent run`. The old advice was the one command that
    // does not survive the terminal it was typed in, so a machine "fixed" that
    // way silently leaves the fleet at the next logout — and every other
    // surface, including the website's own guide, says `install`. Since #112 it
    // also STARTS a service that is merely stopped, which is the state this
    // branch is most often reached in.
    if installed {
        out.push(
            "a service IS installed here but nothing is listening.\n  \
             Start it with:  atlasctl agent install"
                .to_owned(),
        );
    } else {
        out.push(
            "no service is installed on this machine.\n  \
             Install and start one with:  atlasctl agent install"
                .to_owned(),
        );
    }
    out.push(
        "or run it in this terminal, for as long as it stays open:\n  atlasctl agent run"
            .to_owned(),
    );
    out
}

/// Report whether an agent is reachable.
///
/// Probes the port asked for rather than assuming the default. An agent
/// installed with `--port 9000` was reported as "not running", followed by
/// advice to start another one — which then failed to bind against the agent
/// that was running the whole time.
pub fn status(args: &AgentStatusArgs) -> Result<()> {
    let addr = format!("127.0.0.1:{}", args.port);
    match std::net::TcpStream::connect(&addr) {
        Ok(_) => {
            println!("agent: running (listening on {addr})");
            Ok(())
        }
        Err(e) => {
            println!("agent: not running on {addr} ({e})");
            // Best effort: a machine whose home directory cannot be read is a
            // machine we cannot make this claim about, and guessing "installed"
            // either way would send the operator to the wrong command.
            let installed = crate::hostinfo::home_dir().is_ok_and(|home| {
                crate::service::installed_unit(
                    &atlasctl_core::io::StdFileSystem,
                    &home,
                    crate::commands::service::uid_of(&home),
                )
                .is_ok_and(|u| u.is_some())
            });
            for line in status_advice(args.port, installed) {
                println!("{line}");
            }
            bail!("no agent is listening on {addr}")
        }
    }
}

#[cfg(test)]
mod status_tests {
    use super::status_advice;

    /// An operator on the default port may simply not know the flag exists.
    #[test]
    fn the_default_port_suggests_looking_elsewhere_first() {
        let a = status_advice(atlasctl_agent::DEFAULT_PORT, false);
        assert!(
            a.iter().any(|l| l.contains("--port")),
            "must name the flag: {a:?}"
        );
    }

    /// But an operator who NAMED a port has already answered that question.
    /// Repeating it there would be noise, and worse, it would imply the port
    /// they gave was not the one probed.
    #[test]
    fn an_explicit_port_is_not_second_guessed() {
        let a = status_advice(9000, false);
        assert!(
            !a.iter().any(|l| l.contains("--port")),
            "must not re-suggest the flag: {a:?}"
        );
        assert!(
            a.iter().any(|l| l.contains("agent run")),
            "still offers a start"
        );
    }

    /// The two states an operator is actually in, and they need different
    /// commands. Reporting only "not running" made "I installed it and it
    /// stopped" indistinguishable from "I never installed it", and the single
    /// piece of advice offered — `agent run` — was wrong for both: it is the
    /// one command that does not survive the terminal it was typed in.
    #[test]
    fn a_stopped_service_and_a_missing_one_are_told_apart() {
        let stopped = status_advice(9000, true).join("\n");
        let missing = status_advice(9000, false).join("\n");
        assert!(stopped.contains("service IS installed"), "{stopped}");
        assert!(missing.contains("no service is installed"), "{missing}");
        // Both lead with the command that works in either case and persists.
        for a in [&stopped, &missing] {
            assert!(a.contains("atlasctl agent install"), "{a}");
            let install = a.find("agent install").expect("offers install");
            let run = a.find("agent run").expect("still offers the foreground");
            assert!(install < run, "the persistent option must come first: {a}");
        }
    }
}
