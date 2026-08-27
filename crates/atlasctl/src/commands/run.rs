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
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};
use atlasctl_core::registry::RecipeRef;

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
        eprintln!(
            "warning: `{}` is not a setting this version understands; \
             it will NOT be applied (value: {})",
            u.key, u.rendered
        );
    }
    if !plan.unknown_keys.is_empty() {
        eprintln!(
            "warning: recipe has unrecognised top-level keys, ignored: {}",
            plan.unknown_keys.join(", ")
        );
    }

    if args.print {
        if args.portable {
            println!("{}", plan.docker.display_portable(Some(&host.home)));
        } else {
            println!("{}", plan.docker);
        }
        return Ok(());
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
