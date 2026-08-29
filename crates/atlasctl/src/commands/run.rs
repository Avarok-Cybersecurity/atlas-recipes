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
        let shown = if setting_shaped(&u.rendered) {
            u.rendered.as_str()
        } else {
            "withheld"
        };
        eprintln!(
            "warning: `{}` is not a setting this version understands; \
             it will NOT be applied (value: {shown}){hint}",
            u.key
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
    // Checked HERE and not only in `doctor`, because this is where the cost
    // lands: a launch begun on a full disk pulls for forty minutes and then
    // fails with a docker error that never mentions space. A warning rather
    // than a refusal — the weights may already be cached, in which case nothing
    // is pulled and the number is irrelevant.
    if let Some(why) = disk_caution_for(&host.hf_cache_dir) {
        eprintln!("warning: {why}");
    }

    if args.print {
        if args.portable {
            println!(
                "{}",
                plan.docker.display_portable(
                    Some(&host.home),
                    atlasctl_core::platform::home_placeholder()
                )
            );
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
    {
        match hfcache::cache_state(&dir) {
            hfcache::CacheState::Weights => {}
            hfcache::CacheState::Absent => bail!(
                "`{}` needs the model `{}`, which is not in {}.\n\
                     The launch runs offline, so it cannot download it. Fetch it first:\n\
                       hf download {}",
                args.recipe,
                recipe.model,
                host.hf_cache_dir,
                recipe.model
            ),
            // Distinct from Absent on purpose. `hf download` is the same fix,
            // but "it is not there" is the wrong thing to tell someone who can
            // SEE the directory: they check, find it, and conclude the tool is
            // broken. Naming the state — present, but no weights — is what makes
            // the instruction believable.
            hfcache::CacheState::MetadataOnly => bail!(
                "`{}` needs the model `{}`. Its cache directory exists in {}, but \
                 holds no weight files — only metadata, which is what an \
                 interrupted or metadata-only download leaves behind.\n\
                     The launch runs offline, so it cannot fetch the rest. \
                 Complete it with:\n\
                       hf download {}",
                args.recipe,
                recipe.model,
                host.hf_cache_dir,
                recipe.model
            ),
        }
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

/// Whether a dropped value is safe to name in the warning.
///
/// Naming it is the useful half for `-o max_seq_lenn=8192` and the wrong half
/// for `-o hf_tokn=<credential>`: mistyping the KEY does not stop the value
/// being a secret, and a warning goes to stderr — into CI logs and into the
/// output people paste into bug reports. The agent already answers this way,
/// reporting `unapplied` as keys only (`launcher/docker.rs`).
///
/// Matched on the VALUE, not the key. The first attempt here matched the key
/// against "token", "secret" and friends, and failed on the exact case that
/// motivated it: `hf_tokn` does not contain "token", because a typo is what
/// put it in this warning in the first place.
///
/// So the test is inverted — a value is shown only when it looks like a
/// SETTING: a number, a bool, or a short lowercase word of the shape every
/// enum and dtype in the table uses. Anything else is withheld. That errs
/// toward saying less about a value nobody asked us to keep.
///
/// It does not catch everything, and should not claim to: a short all-lowercase
/// password is indistinguishable from a dtype name by shape alone. What it does
/// catch is the shape credentials actually have — long, or mixed case, or
/// carrying characters no setting in the table uses. The remaining case is one
/// the operator has already put in their own shell history.
fn setting_shaped(value: &str) -> bool {
    if value.len() > 16 || value.is_empty() {
        return false;
    }
    value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
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

/// Whether the filesystem that will hold the weights has room for them.
///
/// The HF cache, not the working directory: that is where a model lands, and it
/// is routinely a different filesystem from the one `doctor` reports on.
fn disk_caution_for(hf_cache_dir: &str) -> Option<String> {
    let path = std::path::Path::new(hf_cache_dir);
    let free = atlasctl_core::platform::free_bytes(path)?;
    super::doctor_checks::disk_caution(free, hf_cache_dir)
}

#[cfg(test)]
mod value_shape_tests {
    use super::setting_shaped;

    /// Every value a real setting can take must survive, or the warning stops
    /// being useful for the case it exists for.
    #[test]
    fn the_values_settings_actually_take_are_named() {
        for v in [
            "8192",
            "16",
            "0.85",
            "true",
            "false",
            "auto",
            "bf16",
            "fp8",
            "slai",
            "fifo",
            "nvfp4",
            "1",
            "0",
            "262144",
            "e4m3",
            "flashinfer",
        ] {
            assert!(setting_shaped(v), "{v} is an ordinary setting value");
        }
    }

    /// The shapes credentials actually have.
    #[test]
    fn credential_shaped_values_are_withheld() {
        for v in [
            "hf_SECRETVALUE123",
            "sk-abc123XYZdef456",
            "ghp_AbCdEf0123456789",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9",
            "a-very-long-value-that-goes-on",
        ] {
            assert!(!setting_shaped(v), "{v} must not be echoed");
        }
    }

    #[test]
    fn the_boundaries_are_where_they_are_stated() {
        assert!(
            setting_shaped(&"a".repeat(16)),
            "16 is the limit, inclusive"
        );
        assert!(!setting_shaped(&"a".repeat(17)), "17 is over it");
        assert!(!setting_shaped(""), "nothing to name");
        assert!(
            !setting_shaped("HasUpper"),
            "uppercase is not a setting shape"
        );
        assert!(!setting_shaped("has space"), "nor whitespace");
        assert!(!setting_shaped("semi;colon"), "nor punctuation outside ._-");
    }
}
