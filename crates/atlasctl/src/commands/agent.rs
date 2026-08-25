// SPDX-License-Identifier: AGPL-3.0-only

//! `agent run`, `agent token`, `agent status`.

use crate::cli::{AgentRunArgs, AgentTokenArgs};
use crate::hostinfo;
use anyhow::{Context, Result, bail};
use atlasctl_agent::launcher::DockerLauncher;
use atlasctl_agent::server::{AgentState, serve};
use atlasctl_agent::token;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use std::sync::Arc;

/// Whether this machine can actually run a recipe, and why not if it cannot.
///
/// Probed once at startup and reported to the client, so a browser can say
/// "this box cannot launch" instead of offering a button that will fail. A
/// machine that cannot launch is still useful: it can list and inspect.
fn probe_can_launch(runner: &dyn ProcessRunner) -> Result<(), String> {
    match runner.run(&[
        "docker".into(),
        "info".into(),
        "--format".into(),
        "{{.ServerVersion}}".into(),
    ]) {
        Ok(out) if out.success() => Ok(()),
        Ok(out) => Err(format!(
            "the docker daemon did not answer: {}",
            out.stderr.trim()
        )),
        Err(e) => Err(format!("docker is not available: {e}")),
    }
}

/// Run the agent in the foreground.
pub fn run(args: &AgentRunArgs) -> Result<()> {
    let config_dir = hostinfo::config_dir()?;
    let tok = token::load_or_create(&config_dir)?;
    let runner: Arc<dyn ProcessRunner> = Arc::new(StdProcessRunner);
    let can_launch = probe_can_launch(runner.as_ref());

    let state = Arc::new(AgentState {
        registry: crate::commands::registry_set()?,
        launcher: Box::new(DockerLauncher::new(
            runner,
            hostinfo::snapshot()?,
            &ROOTLESS_V1,
            Box::new(NvidiaDevices),
        )),
        token: tok.clone(),
        can_launch: can_launch.clone(),
        port: args.port,
        allow_dev_origins: args.dev_origins,
    });

    eprintln!("atlasctl agent listening on 127.0.0.1:{}", args.port);
    match &can_launch {
        Ok(()) => eprintln!("docker: ok"),
        Err(why) => eprintln!(
            "docker: unavailable — {why}\n  this agent can list and inspect recipes but not launch them"
        ),
    }
    if args.dev_origins {
        eprintln!("accepting development origins — do not leave this on");
    }
    eprintln!("\npairing token (paste into the website once):\n  {tok}\n");
    eprintln!("This agent talks to Docker. On Linux, membership of the `docker` group is");
    eprintln!("root-equivalent, so anything that can drive this agent can do what you can.");
    eprintln!("Stop it with ctrl-c when you are done.\n");

    let rt = tokio::runtime::Builder::new_multi_thread()
        // Two workers is ample: this serves one local browser, not a fleet.
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    rt.block_on(serve(state, args.port))
}

/// Print, or rotate, the pairing token.
pub fn token(args: &AgentTokenArgs) -> Result<()> {
    let dir = hostinfo::config_dir()?;
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

/// Report whether an agent is reachable on the default port.
pub fn status() -> Result<()> {
    let addr = format!("127.0.0.1:{}", atlasctl_agent::DEFAULT_PORT);
    match std::net::TcpStream::connect(&addr) {
        Ok(_) => {
            println!("agent: running (listening on {addr})");
            Ok(())
        }
        Err(e) => {
            println!("agent: not running ({e})");
            println!("start it with: atlasctl agent run");
            bail!("no agent is listening on {addr}")
        }
    }
}
