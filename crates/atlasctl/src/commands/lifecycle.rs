// SPDX-License-Identifier: AGPL-3.0-only

//! `stop`, `logs`, `status`.

use crate::cli::{LogsArgs, StopArgs};
use anyhow::{Result, bail};
use atlasctl_core::docker::translate::{LABEL_MANAGED, LABEL_RECIPE};
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use atlasctl_core::registry::RecipeRef;

/// Container name for a recipe's solo launch.
fn container_of(recipe: &str) -> String {
    format!("atlas-{recipe}")
}

/// Stop one recipe, or everything atlasctl started.
pub fn stop(args: &StopArgs) -> Result<()> {
    if args.all {
        return stop_all_with(&StdProcessRunner);
    }
    let Some(typed) = &args.recipe else {
        bail!("name a recipe to stop, or pass --all");
    };
    match known_recipe(typed) {
        Ok(resolved) => stop_recipe_with(&StdProcessRunner, typed, &resolved),
        // The catalogue could not answer — an ambiguous bare name, a recipe
        // whose YAML no longer parses, an unreadable registries.yaml. None of
        // that should strand a container this fleet is running: the label was
        // written at launch and does not depend on the catalogue still being
        // readable. Only when nothing is running under that name does the
        // resolve error stand, so a TYPO still gets its "Did you mean ...?".
        Err(unresolved) => {
            let found = containers_for_recipe(&StdProcessRunner, typed)?;
            if found.is_empty() {
                return Err(unresolved);
            }
            stop_each(&StdProcessRunner, found)
        }
    }
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
fn known_recipe(name: &str) -> Result<String> {
    Ok(crate::commands::registry_set()?
        .resolve(&RecipeRef::parse(name))?
        .name)
}

/// The running containers this fleet launched FOR a recipe, by label.
///
/// Not `atlas-{typed}`: the container is named from the resolved recipe and a
/// cluster launch appends `-rank{n}` (`docker::translate::container_name`), so
/// guessing missed two everyday cases and — once "no such container" stopped
/// being a failure — reported exit 0 while the model served:
///
///   * `run X --rank 0` makes `atlas-X-rank0`, and `run` prints
///     `atlasctl stop X` as the way to stop it;
///   * `stop @registry/X` guessed `atlas-@registry/X`.
///
/// The label is written by the launch itself, so it survives a registry being
/// removed and needs no name arithmetic here.
fn containers_for_recipe(runner: &dyn ProcessRunner, recipe: &str) -> Result<Vec<String>> {
    let out = runner.run(&[
        "docker".into(),
        "ps".into(),
        "--filter".into(),
        format!("label={LABEL_RECIPE}={recipe}"),
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

/// `stop --all`, with the runner injected so the failure paths are testable.
fn stop_all_with(runner: &dyn ProcessRunner) -> Result<()> {
    let targets = managed_containers(runner)?;
    if targets.is_empty() {
        println!("nothing running");
        return Ok(());
    }
    stop_each(runner, targets)
}

/// `stop <recipe>`, found by label rather than by guessing a container name.
///
/// `typed` is what the operator wrote (for the message); `resolved` is the
/// recipe's own name, which is what the launch wrote into the label.
fn stop_recipe_with(runner: &dyn ProcessRunner, typed: &str, resolved: &str) -> Result<()> {
    let found = containers_for_recipe(runner, resolved)?;
    if found.is_empty() {
        println!("{typed} is not running");
        return Ok(());
    }
    stop_each(runner, found)
}

/// Stop each container, collecting real failures.
fn stop_each(runner: &dyn ProcessRunner, targets: Vec<String>) -> Result<()> {
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
    let mut name = name;
    if probe.success() && probe.stdout.trim().is_empty() {
        // The exact name missed. Ask the LABEL before concluding nothing is
        // running, for the reason `containers_for_recipe` documents: a cluster
        // launch appends `-rank{n}`, and a registry-qualified name produces
        // `atlas-@registry/X`, which docker cannot even hold. `stop` was fixed
        // for exactly these two cases and `logs` was not -- so `run X --rank 0`
        // printed "started atlas-X-rank0" and, on the very next line,
        // "logs: atlasctl logs X --follow", a command that then denied the
        // container existed.
        let found = containers_for_recipe(runner, &args.recipe)?;
        match found.len() {
            0 => bail!(
                "no container for `{}` on this machine — it has not been started here. `atlasctl status` lists what is running",
                args.recipe
            ),
            1 => name = found.into_iter().next().unwrap_or_default(),
            // Several ranks on this box. Naming one for the operator would be a
            // guess about which one they meant, and the ranks do not log the
            // same thing; say what is there instead.
            _ => bail!(
                "`{}` is running as {} containers on this machine: {}. \
                 Read one with:  docker logs --tail 200 -f <name>",
                args.recipe,
                found.len(),
                found.join(", ")
            ),
        }
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
    // An empty list because docker did not ANSWER is not an idle fleet. Without
    // this, `stop --all` against a stopped daemon printed "nothing running" and
    // exited 0 — the exact lie the collected-failures logic above exists to
    // prevent. `status()` ten lines down has always checked this.
    if !out.success() {
        bail!("`docker ps` failed: {}", out.stderr.trim());
    }
    Ok(out
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

#[cfg(test)]
mod tests;
