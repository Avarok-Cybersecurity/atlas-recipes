// SPDX-License-Identifier: AGPL-3.0-only

//! `stop`, `logs`, `status`.

use crate::cli::{LogsArgs, StopArgs};
use anyhow::{Result, bail};
use atlasctl_core::docker::translate::LABEL_MANAGED;
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};

/// Container name for a recipe's solo launch.
fn container_of(recipe: &str) -> String {
    format!("atlas-{recipe}")
}

/// Stop one recipe, or everything atlasctl started.
pub fn stop(args: &StopArgs) -> Result<()> {
    stop_with(&StdProcessRunner, args)
}

/// `stop`, with the process runner injected so the failure paths are testable.
///
/// The interesting behaviour here is entirely about what happens when docker
/// says no, and that cannot be reached through a real `docker` without one.
fn stop_with(runner: &dyn ProcessRunner, args: &StopArgs) -> Result<()> {
    let targets: Vec<String> = if args.all {
        managed_containers(runner)?
    } else {
        let Some(recipe) = &args.recipe else {
            bail!("name a recipe to stop, or pass --all");
        };
        vec![container_of(recipe)]
    };

    if targets.is_empty() {
        println!("nothing running");
        return Ok(());
    }
    // Failures are collected rather than only printed. Reporting each one to
    // stderr and then returning Ok meant `atlasctl stop --all` exited 0 with
    // every container still running — so a script that checked the exit code,
    // or an operator who ran it before a reboot, was told the fleet was idle
    // when it was not.
    let mut failed: Vec<String> = Vec::new();
    for name in targets {
        let out = runner.run(&["docker".into(), "stop".into(), name.clone()])?;
        if out.success() {
            println!("stopped {name}");
        } else {
            let why = out.stderr.trim();
            eprintln!("could not stop {name}: {why}");
            failed.push(name);
        }
    }
    if !failed.is_empty() {
        // Named, because "1 of 3 failed" sends the operator to check all three.
        bail!(
            "could not stop {} container(s): {}",
            failed.len(),
            failed.join(", ")
        );
    }
    Ok(())
}

/// Follow or tail a recipe's logs.
///
/// Because a launch is a single `docker run` ending in `spark serve`, the serve
/// process is PID 1 and `docker logs` shows its output directly. The tool this
/// replaces ran `sleep infinity` as PID 1, so its `docker logs` showed nothing
/// and it had to tail a file inside the container instead.
pub fn logs(args: &LogsArgs) -> Result<()> {
    let mut argv = vec![
        "docker".to_string(),
        "logs".to_string(),
        "--tail".to_string(),
        args.tail.to_string(),
    ];
    if args.follow {
        argv.push("--follow".to_string());
    }
    argv.push(container_of(&args.recipe));
    let code = StdProcessRunner.run_streaming(&argv)?;
    if code != 0 {
        bail!("`docker logs` exited with status {code}");
    }
    Ok(())
}

/// Show what atlasctl has running.
pub fn status() -> Result<()> {
    let runner = StdProcessRunner;
    let out = runner.run(&[
        "docker".into(),
        "ps".into(),
        "--filter".into(),
        format!("label={LABEL_MANAGED}=1"),
        "--format".into(),
        "{{.Names}}\t{{.Status}}\t{{.Image}}".into(),
    ])?;
    if !out.success() {
        bail!("`docker ps` failed: {}", out.stderr.trim());
    }
    let rows: Vec<&str> = out
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if rows.is_empty() {
        println!("nothing running");
        return Ok(());
    }
    println!("{:<40}  {:<24}  IMAGE", "CONTAINER", "STATUS");
    for row in rows {
        let mut parts = row.split('\t');
        println!(
            "{:<40}  {:<24}  {}",
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or("")
        );
    }
    Ok(())
}

/// Names of every container atlasctl started.
fn managed_containers(runner: &dyn ProcessRunner) -> Result<Vec<String>> {
    let out = runner.run(&[
        "docker".into(),
        "ps".into(),
        "--filter".into(),
        format!("label={LABEL_MANAGED}=1"),
        "--format".into(),
        "{{.Names}}".into(),
    ])?;
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlasctl_core::docker::translate::LABEL_RECIPE;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    #[test]
    fn managed_containers_are_found_by_label_not_by_name_guessing() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        let names = managed_containers(&r).unwrap();
        assert_eq!(names, ["atlas-a", "atlas-b"]);
        let argv = &r.calls()[0];
        assert!(
            argv.contains(&format!("label={LABEL_MANAGED}=1")),
            "must filter by our label so unrelated containers are never touched: {argv:?}"
        );
    }

    #[test]
    fn the_recipe_label_is_what_ties_a_container_back_to_its_recipe() {
        assert_eq!(LABEL_RECIPE, "io.atlasctl.recipe");
        assert_eq!(container_of("qwen3.6-27b-fp8"), "atlas-qwen3.6-27b-fp8");
    }
}

#[cfg(test)]
mod stop_tests {
    use super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    fn ok() -> Output {
        Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
    fn fail(why: &str) -> Output {
        Output {
            status: 1,
            stdout: String::new(),
            stderr: why.into(),
        }
    }

    /// The bug: every stop failed and the command exited 0, so a script that
    /// checked the exit code — or an operator running this before a reboot —
    /// was told the fleet was idle while every container was still up.
    #[test]
    fn a_failed_stop_is_a_failed_command() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        r.push_result(fail("permission denied"));
        r.push_result(fail("no such container"));

        let e = stop_with(
            &r,
            &StopArgs {
                recipe: None,
                all: true,
            },
        )
        .expect_err("two failures must not report success");
        let msg = e.to_string();
        assert!(msg.contains("atlas-a") && msg.contains("atlas-b"), "{msg}");
    }

    /// A partial failure is still a failure, and the survivors are named —
    /// "1 of 3 failed" sends the operator to check all three.
    #[test]
    fn a_partial_failure_names_only_what_did_not_stop() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        r.push_result(ok());
        r.push_result(fail("device busy"));

        let e = stop_with(
            &r,
            &StopArgs {
                recipe: None,
                all: true,
            },
        )
        .expect_err("one failure is a failure");
        let msg = e.to_string();
        assert!(msg.contains("atlas-b"), "{msg}");
        assert!(
            !msg.contains("atlas-a"),
            "the one that stopped is not a problem: {msg}"
        );
    }

    #[test]
    fn stopping_everything_when_nothing_runs_is_success() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        stop_with(
            &r,
            &StopArgs {
                recipe: None,
                all: true,
            },
        )
        .expect("nothing to do is fine");
    }
}
