// SPDX-License-Identifier: AGPL-3.0-only

//! Installing the agent so it is there after a reboot.
//!
//! The website's whole premise is that a machine you own is reachable from a
//! page you did not have to trust. That only holds if the agent is running, and
//! an agent started by hand in a terminal stops being there the moment the
//! terminal closes — which is not a fleet, it is a demo.
//!
//! Everything with a decision in it lives in [`plan`], which is pure. This file
//! is the I/O: write the unit, run the commands, report what happened. Both the
//! filesystem and the process runner are injected, so the orderings that matter
//! — reload before enable, disable before delete — are tested without a
//! systemd anywhere near the test.
//!
//! **This does not raise privilege.** The unit is a `systemd --user` unit and
//! the plist a LaunchAgent; both run as the installing user. Nothing here is
//! written to a system path and nothing asks for root. That is deliberate: the
//! agent already has docker access, which on Linux is root-equivalent, and a
//! root-owned service on top of that would widen a surface that is quite wide
//! enough.

pub mod plan;

#[cfg(test)]
mod tests;

use anyhow::{Context, Result};
use atlasctl_core::io::{FileSystem, ProcessRunner};
use plan::{AgentInvocation, ServiceKind, ServicePlan};
use std::path::PathBuf;

/// What an install did, so the caller can describe it rather than guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// Where the unit was written.
    pub unit_path: PathBuf,
    /// Which supervisor now owns the agent.
    pub kind: ServiceKind,
    /// Steps that were allowed to fail and did, with the reason.
    ///
    /// Reported rather than swallowed: on a headless machine a failed
    /// `enable-linger` is the difference between an agent that survives logout
    /// and one that does not, and the operator can only act on it if they are
    /// told.
    pub skipped: Vec<String>,
}

/// Install the agent as a background service for the current user.
///
/// # Errors
/// If the unit cannot be written, or a required activation step fails.
pub fn install(
    fs: &dyn FileSystem,
    runner: &dyn ProcessRunner,
    agent: &AgentInvocation,
    home: &std::path::Path,
    uid: u32,
) -> Result<Installed> {
    let kind = ServiceKind::detect()?;
    let p = plan::plan(kind, agent, home, uid);

    let parent = p
        .unit_path
        .parent()
        .context("the unit path has no directory")?;
    fs.create_dir_all(parent)?;
    fs.write_atomic(&p.unit_path, &p.unit_body)?;

    for argv in &p.activate {
        let out = runner.run(argv)?;
        if !out.success() {
            // The unit is left on disk on purpose. It is the evidence for why
            // the step failed, and deleting it would turn a diagnosable
            // failure into "the install did nothing".
            anyhow::bail!(
                "`{}` failed: {}",
                argv.join(" "),
                first_line(&out.stderr, &out.stdout)
            );
        }
    }

    let mut skipped = Vec::new();
    for argv in &p.best_effort {
        match runner.run(argv) {
            Ok(out) if out.success() => {}
            Ok(out) => skipped.push(format!(
                "{}: {}",
                argv.join(" "),
                first_line(&out.stderr, &out.stdout)
            )),
            Err(e) => skipped.push(format!("{}: {e}", argv.join(" "))),
        }
    }

    Ok(Installed {
        unit_path: p.unit_path,
        kind: p.kind,
        skipped,
    })
}

/// Remove the service. Removing one that is not installed is success.
///
/// # Errors
/// If the platform is unsupported or the unit cannot be deleted.
pub fn uninstall(
    fs: &dyn FileSystem,
    runner: &dyn ProcessRunner,
    home: &std::path::Path,
    uid: u32,
) -> Result<PathBuf> {
    let kind = ServiceKind::detect()?;
    // The invocation does not affect where the unit lives or how it is torn
    // down, and inventing one here would be a fabricated value in a production
    // path. These are the fields the teardown never reads.
    let p = teardown_plan(kind, home, uid);

    // Stop it before deleting the file: a supervisor asked to disable a unit
    // whose file has already gone can refuse, leaving the service running with
    // nothing on disk to stop it with.
    for argv in &p.deactivate {
        // A service that was never installed makes this fail, and that is the
        // ordinary case for `uninstall` run twice.
        let _ = runner.run(argv);
    }
    fs.remove_file(&p.unit_path)?;

    if p.kind == ServiceKind::Systemd {
        let _ = runner.run(&[
            "systemctl".to_owned(),
            "--user".to_owned(),
            "daemon-reload".to_owned(),
        ]);
    }
    Ok(p.unit_path)
}

/// A plan built for teardown, where the agent's flags are irrelevant.
fn teardown_plan(kind: ServiceKind, home: &std::path::Path, uid: u32) -> ServicePlan {
    plan::plan(
        kind,
        &AgentInvocation {
            exe: PathBuf::from("atlasctl"),
            port: 0,
            client: false,
            discovery: true,
            browser: true,
            config_dir: None,
        },
        home,
        uid,
    )
}

/// The first useful line of a failed command, for a message worth reading.
fn first_line(stderr: &str, stdout: &str) -> String {
    for s in [stderr, stdout] {
        if let Some(l) = s.lines().map(str::trim).find(|l| !l.is_empty()) {
            return l.to_owned();
        }
    }
    "no output".to_owned()
}
