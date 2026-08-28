// SPDX-License-Identifier: AGPL-3.0-only

//! `stop`, `logs`, `status`.

use crate::cli::{LogsArgs, StopArgs};
use anyhow::{Result, bail};
use atlasctl_core::docker::translate::LABEL_MANAGED;
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use atlasctl_core::registry::RecipeRef;

/// Container name for a recipe's solo launch.
fn container_of(recipe: &str) -> String {
    format!("atlas-{recipe}")
}

/// Stop one recipe, or everything atlasctl started.
pub fn stop(args: &StopArgs) -> Result<()> {
    if let Some(recipe) = &args.recipe {
        known_recipe(recipe)?;
    }
    stop_with(&StdProcessRunner, args)
}

/// Refuse a name that is not a recipe at all.
///
/// Without this, "not running" answers a TYPO as readily as a real recipe, and
/// since that is no longer a failure, `atlasctl stop $RECIPE` with a misspelt
/// variable would report success having stopped nothing. Resolving through the
/// registry also means a near miss gets the same "Did you mean ...?" list the
/// rest of the CLI gives.
///
/// `--all` skips this: its targets come from docker's own list of containers we
/// label, so they need no catalogue entry. That is also the way out if a recipe
/// is running from a registry that has since been removed.
fn known_recipe(name: &str) -> Result<()> {
    crate::commands::registry_set()?.resolve(&RecipeRef::parse(name))?;
    Ok(())
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
        } else if absent(&out.stderr) {
            // Not a failure: the state the operator asked for is already true.
            // This printed "could not stop atlas-x: Error response from daemon:
            // No such container: atlas-x" and then exited non-zero, sending
            // someone to inspect docker over a recipe that was simply never
            // started. Under --all it is a race -- the container ended between
            // the listing and the stop -- which is equally not a failure.
            println!("{name} is not running");
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
    known_recipe(&args.recipe)?;
    logs_with(&StdProcessRunner, args)
}

/// `logs`, with the runner injected so the not-started path is testable.
fn logs_with(runner: &dyn ProcessRunner, args: &LogsArgs) -> Result<()> {
    let name = container_of(&args.recipe);

    // Ask whether the container exists before streaming. `docker logs` on a
    // container that is not there prints the daemon's own line and exits 1,
    // which surfaced as "`docker logs` exited with status 1" -- accurate, and
    // no help to anyone. Asked as a filter rather than by matching the error
    // text, because the daemon words it differently per command: `stop` and
    // `logs` say "No such container", `inspect` says "no such object".
    //
    // `ps -a`, not `ps`: a container that exited still has logs worth reading,
    // and that is often exactly why someone is here.
    let probe = runner.run(&[
        "docker".into(),
        "ps".into(),
        "-a".into(),
        "--filter".into(),
        format!("name=^{name}$"),
        "--format".into(),
        "{{.Names}}".into(),
    ])?;
    if probe.success() && probe.stdout.trim().is_empty() {
        bail!(
            "no container for `{}` on this machine — it has not been started here. `atlasctl status` lists what is running",
            args.recipe
        );
    }

    let mut argv = vec![
        "docker".to_string(),
        "logs".to_string(),
        "--tail".to_string(),
        args.tail.to_string(),
    ];
    if args.follow {
        argv.push("--follow".to_string());
    }
    argv.push(name);
    let code = runner.run_streaming(&argv)?;
    if code != 0 {
        bail!("`docker logs` exited with status {code}");
    }
    Ok(())
}

/// Whether the daemon is saying the container is not there.
///
/// Matched on text because `docker stop` exits 1 for every failure and carries
/// the distinction only in stderr. Lowercased and checked for both spellings,
/// since the wording is not stable across commands or versions.
fn absent(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("no such container") || s.contains("no such object")
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
        // Was "no such container", which is now the one stderr that means the
        // stop SUCCEEDED in effect. This test is about two real failures, so it
        // needs two real failures.
        r.push_result(fail("device or resource busy"));

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

#[cfg(test)]
mod absent_tests {
    use super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    fn out(status: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn the_daemons_two_spellings_of_gone_are_both_recognised() {
        assert!(absent(
            "Error response from daemon: No such container: atlas-x"
        ));
        assert!(absent("Error: no such object: atlas-x"));
        // Real failures must not be swallowed by this.
        assert!(!absent("permission denied"));
        assert!(!absent("device or resource busy"));
        assert!(!absent(""));
    }

    /// Stopping something that is not running is the state the operator asked
    /// for. It used to print the daemon's own line and exit non-zero.
    #[test]
    fn stopping_a_recipe_that_is_not_running_is_not_a_failure() {
        let r = RecordingRunner::new();
        r.push_result(out(
            1,
            "",
            "Error response from daemon: No such container: atlas-q",
        ));
        stop_with(
            &r,
            &StopArgs {
                recipe: Some("q".into()),
                all: false,
            },
        )
        .expect("a recipe that is not running must not be an error");
    }

    /// Under --all this is a race: the container ended between the listing and
    /// the stop. Equally not a failure, and it must not mask a real one.
    #[test]
    fn a_vanished_container_does_not_mask_a_real_failure() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-a\natlas-b\n", ""));
        r.push_result(out(
            1,
            "",
            "Error response from daemon: No such container: atlas-a",
        ));
        r.push_result(out(1, "", "permission denied"));
        let e = stop_with(
            &r,
            &StopArgs {
                recipe: None,
                all: true,
            },
        )
        .expect_err("the real failure must still fail the command");
        let msg = e.to_string();
        assert!(msg.contains("atlas-b"), "names what actually failed: {msg}");
        assert!(
            !msg.contains("atlas-a"),
            "must not blame the one that was already gone: {msg}"
        );
    }

    #[test]
    fn logs_for_a_recipe_never_started_here_says_so_and_never_streams() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "\n", "")); // ps -a matched nothing
        let e = logs_with(
            &r,
            &LogsArgs {
                recipe: "q".into(),
                tail: 100,
                follow: false,
            },
        )
        .expect_err("there is nothing to show");
        let msg = e.to_string();
        assert!(msg.contains("has not been started here"), "{msg}");
        assert!(
            msg.contains("atlasctl status"),
            "points somewhere useful: {msg}"
        );
        assert_eq!(r.calls().len(), 1, "must not have run `docker logs` at all");
    }

    #[test]
    fn logs_for_an_exited_container_still_stream() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-q\n", "")); // ps -a found it, exited or not
        r.push_result(out(0, "", ""));
        logs_with(
            &r,
            &LogsArgs {
                recipe: "q".into(),
                tail: 100,
                follow: false,
            },
        )
        .expect("a stopped container still has logs worth reading");
        let argv = &r.calls()[1];
        assert!(argv.contains(&"logs".to_string()), "{argv:?}");
    }
}
