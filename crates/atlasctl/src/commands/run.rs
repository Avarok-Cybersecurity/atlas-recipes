// SPDX-License-Identifier: AGPL-3.0-only

//! `run` — launch a recipe.

use crate::cli::RunArgs;
use crate::hostinfo;
use crate::validate;
use anyhow::{Result, bail};
use atlasctl_core::chain::UserConfig;
use atlasctl_core::docker::collective::NcclRoce;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::docker::translate::{LaunchContext, translate};
use atlasctl_core::hfcache;
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use atlasctl_core::nearest;
use atlasctl_core::registry::RecipeRef;
use std::path::Path;

/// Launch a recipe, or print the command it would run.
pub fn run(args: &RunArgs) -> Result<()> {
    let set = crate::commands::registry_set()?;
    let mut recipe = set.resolve(&RecipeRef::parse(&args.recipe))?;

    if let Some(image) = &args.image {
        recipe.container = image.clone();
    }

    let overrides = validate::build_overrides(&args.options, args.port)?;
    let placement = validate::build_placement(
        args.rank,
        args.world_size,
        args.master_addr.clone(),
        args.master_port,
    )?;
    let host = hostinfo::snapshot()?;
    let ctx = LaunchContext {
        profile: &ROOTLESS_V1,
        devices: &NvidiaDevices,
        collective: &NcclRoce,
    };

    let mut plan = translate(
        &recipe,
        &overrides,
        &UserConfig::new(),
        &host,
        &placement,
        &ctx,
    )?;
    if args.no_rm {
        plan.docker.auto_remove = false;
    }

    // Say what will not be applied before doing anything, not after. The tool
    // this replaces dropped these settings in silence.
    for u in &plan.unmapped {
        // Two different mistakes, answered differently. Reading the rendered
        // command and typing what you saw is not a typo -- `--max-seq-len` is
        // the key `max_model_len` -- and no distance ranking finds it, so the
        // table that holds both spellings answers it exactly. Anything else
        // gets the near-miss list.
        let hint = if let Some(key) = atlasctl_core::flags::key_for_flag_spelling(&u.key) {
            format!(". That is the engine's flag name; the setting key is `{key}`")
        } else {
            nearest::did_you_mean(&nearest::nearest(&u.key, atlasctl_core::flags::keys()))
        };
        eprintln!(
            "warning: `{}` is not a setting this version understands; \
             it will NOT be applied (value: {}){hint}",
            u.key, u.rendered
        );
    }
    if !plan.unknown_keys.is_empty() {
        eprintln!(
            "warning: recipe has unrecognised top-level keys, ignored: {}",
            plan.unknown_keys.join(", ")
        );
    }

    // Values that are allowed and still worth a word. Said BEFORE the pull and
    // the launch, alongside the other pre-flight warnings, because afterwards
    // the machine may not be answering.
    for c in cautions(&plan.docker.command) {
        eprintln!("warning: {c}");
    }

    if args.print {
        if args.portable {
            println!("{}", plan.docker.display_portable(Some(&host.home)));
        } else {
            println!("{}", plan.docker);
        }
        return Ok(());
    }

    // The container is launched with HF_HUB_OFFLINE=1, so a model that is not
    // already in the host cache cannot be fetched from inside it. Said here,
    // before the image pull, because otherwise the operator waits through a
    // multi-gigabyte pull to reach the Hub library's own cache-miss message,
    // inside a container that has already exited, reachable only via
    // `atlasctl logs`.
    //
    // Unless the launch brings its own weights: `model_from_path` points the
    // engine at a directory, and then the Hub cache is not consulted at all, so
    // refusing on a cache miss would block a launch that was going to work.
    // Read off the rendered argv rather than the override map, because a recipe
    // can set it in `defaults:` and never mention it on the command line.
    let brings_own_weights = plan.docker.command.iter().any(|a| a == "--model-from-path");
    if !brings_own_weights
        && let Some(dir) = hfcache::hub_dir(Path::new(&host.hf_cache_dir), &recipe.model)
        && !dir.exists()
    {
        bail!(
            "`{}` needs the model `{}`, which is not in {}.\n\
                 The launch runs offline, so it cannot download it. Fetch it first:\n\
                   hf download {}",
            args.recipe,
            recipe.model,
            host.hf_cache_dir,
            recipe.model
        );
    }

    let runner = StdProcessRunner;

    if !args.no_pull {
        eprintln!("pulling {} ...", plan.docker.image);
        let pull = vec![
            "docker".to_string(),
            "pull".to_string(),
            plan.docker.image.clone(),
        ];
        let code = runner.run_streaming(&pull)?;
        if code != 0 {
            bail!(
                "`docker pull {}` failed with status {code}",
                plan.docker.image
            );
        }
    }

    // Clear a previous container of the same name. Failure is expected and
    // ignored when nothing is there; the launch below is what must succeed.
    let _ = runner.run(&[
        "docker".to_string(),
        "rm".to_string(),
        "-f".to_string(),
        plan.docker.name.clone(),
    ]);

    let argv = plan.docker.to_argv();
    let out = runner.run(&argv)?;
    if !out.success() {
        bail!(
            "`docker run` failed with status {}:\n{}",
            out.status,
            out.stderr.trim()
        );
    }

    let port = plan
        .docker
        .command
        .windows(2)
        .find(|w| w[0] == "--port")
        .map(|w| w[1].clone())
        // The engine's own default, asserted against the vendored snapshot of its
        // clap definition rather than copied on faith — this value is printed as
        // a URL the operator is invited to open.
        .unwrap_or_else(|| atlasctl_core::flags::DEFAULT_SERVE_PORT.to_string());

    println!("started {}", plan.docker.name);
    if port != "0" {
        println!("endpoint: http://localhost:{port}/v1");
    }
    println!("logs:     atlasctl logs {} --follow", recipe.name);
    println!("stop:     atlasctl stop {}", recipe.name);
    Ok(())
}

/// Cautions for the values this launch will actually use.
///
/// Read from the RENDERED command, not from the recipe or the overrides. That
/// is the only place recipe defaults, `--options` and placement have already
/// been resolved against each other, so it is the only place that describes
/// what will actually run. The same technique the endpoint port uses below.
///
/// The accelerator comes from the machine, because the warning is about the
/// hardware the container will run on and a recipe written for one card is
/// routinely run on another.
fn cautions(argv: &[String]) -> Vec<String> {
    let accelerator =
        atlasctl_agent::telemetry::accelerator_name(&StdProcessRunner).unwrap_or_default();
    if accelerator.is_empty() {
        return Vec::new();
    }
    argv.windows(2)
        .filter_map(|w| {
            let key = w[0].strip_prefix("--")?.replace('-', "_");
            let value: f64 = w[1].parse().ok()?;
            atlasctl_core::settings::caution(&key, value, &accelerator)
        })
        .collect()
}
