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
    let runner = StdProcessRunner;
    let targets: Vec<String> = if args.all {
        managed_containers(&runner)?
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
    for name in targets {
        let out = runner.run(&["docker".into(), "stop".into(), name.clone()])?;
        if out.success() {
            println!("stopped {name}");
        } else {
            eprintln!("could not stop {name}: {}", out.stderr.trim());
        }
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
