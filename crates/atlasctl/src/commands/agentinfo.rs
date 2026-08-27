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
/// kernel can refuse a connection.
#[must_use]
pub fn status_advice(port: u16) -> Vec<String> {
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
    out.push("or start it with: atlasctl agent run".to_owned());
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
            for line in status_advice(args.port) {
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
        let a = status_advice(atlasctl_agent::DEFAULT_PORT);
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
        let a = status_advice(9000);
        assert!(
            !a.iter().any(|l| l.contains("--port")),
            "must not re-suggest the flag: {a:?}"
        );
        assert!(
            a.iter().any(|l| l.contains("agent run")),
            "still offers a start"
        );
    }
}
