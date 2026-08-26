// SPDX-License-Identifier: AGPL-3.0-only

//! The golden command corpus.
//!
//! Every launchable recipe is translated under a fixed host snapshot and
//! compared against a committed rendering. This is the anchor of the whole
//! port: the risk in reimplementing a launcher is not that it breaks loudly,
//! it is that it emits a *plausible* command that differs from the one people
//! have been benchmarking against. Freezing the output makes any future change
//! to the flag table or launch profile show up in review as a diff of real
//! commands.
//!
//! Regenerate deliberately with `ATLASCTL_BLESS=1 cargo test --test golden`,
//! and read the resulting diff before committing it.

use atlasctl_core::chain::{Overrides, UserConfig};
use atlasctl_core::docker::collective::NcclRoce;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::docker::translate::{DEFAULT_MASTER_PORT, LaunchContext, Placement, translate};
use atlasctl_core::host::HostSnapshot;
use atlasctl_core::recipe::{Provenance, Recipe};
use atlasctl_core::scalar::ScalarValue;
use atlasctl_core::settings::{self, Disposition};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A host that does not exist, so goldens are identical on every machine.
fn fixed_host() -> HostSnapshot {
    HostSnapshot {
        uid: 1000,
        gid: 1000,
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
        env: BTreeMap::new(),
    }
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn bless() -> bool {
    std::env::var_os("ATLASCTL_BLESS").is_some()
}

/// Render every placement a recipe supports into one text block.
fn render(recipe: &Recipe) -> String {
    let placements: Vec<(String, Placement)> = if recipe.topology.is_multi_node() {
        let world = recipe.topology.min_nodes as u16;
        (0..world)
            .map(|rank| {
                (
                    format!("rank{rank}"),
                    Placement::Rank {
                        rank,
                        world_size: world,
                        master_addr: "10.10.10.1".into(),
                        master_port: DEFAULT_MASTER_PORT,
                    },
                )
            })
            .collect()
    } else {
        vec![("solo".to_string(), Placement::Solo)]
    };

    let mut out = String::new();
    for (label, placement) in placements {
        let ctx = LaunchContext {
            profile: &ROOTLESS_V1,
            devices: &NvidiaDevices,
            collective: &NcclRoce,
        };
        let plan = translate(
            recipe,
            &Overrides::new(),
            &UserConfig::new(),
            &fixed_host(),
            &placement,
            &ctx,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));

        out.push_str(&format!("# {label} — shell\n{}\n\n", plan.docker));
        out.push_str(&format!(
            "# {label} — portable\n{}\n\n",
            plan.docker.display_portable(Some("/home/spark"))
        ));
        out.push_str(&format!(
            "# {label} — argv\n{}\n\n",
            plan.docker.to_argv().join("\n")
        ));
        if !plan.unmapped.is_empty() {
            let keys: Vec<&str> = plan.unmapped.iter().map(|u| u.key.as_str()).collect();
            out.push_str(&format!("# {label} — NOT APPLIED: {}\n\n", keys.join(", ")));
        }
    }
    out
}

#[test]
fn rendered_commands_match_their_goldens() {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("golden dir");

    let mut diffs = Vec::new();
    let mut written = 0usize;

    for e in atlas_recipes_data::all() {
        let prov = Provenance::Builtin {
            path: e.path.to_string(),
        };
        let Ok(recipe) = Recipe::parse(e.name, e.yaml, prov) else {
            continue;
        };
        if !recipe.is_launchable() {
            continue;
        }

        let actual = render(&recipe);
        let path = dir.join(format!("{}.txt", recipe.name));

        if bless() {
            std::fs::write(&path, &actual).expect("write golden");
            written += 1;
            continue;
        }

        match std::fs::read_to_string(&path) {
            Ok(expected) if expected == actual => {}
            Ok(expected) => diffs.push(format!(
                "  {}: rendering changed\n    expected {} bytes, got {}",
                recipe.name,
                expected.len(),
                actual.len()
            )),
            Err(_) => diffs.push(format!(
                "  {}: no golden. If this recipe is new, run \
                 `ATLASCTL_BLESS=1 cargo test --test golden` and review the diff.",
                recipe.name
            )),
        }
    }

    if bless() {
        eprintln!("blessed {written} goldens — review the diff before committing");
        return;
    }

    assert!(
        diffs.is_empty(),
        "rendered commands drifted from their goldens:\n{}\n\n\
         If the change is intended, re-bless and review it as a diff of real commands.",
        diffs.join("\n")
    );
}

#[test]
fn no_golden_is_orphaned() {
    // A golden with no recipe means a recipe was renamed or removed and the
    // stale file would otherwise sit there asserting nothing.
    let dir = golden_dir();
    if !dir.exists() {
        return;
    }
    let names: Vec<String> = atlas_recipes_data::all()
        .iter()
        .map(|e| e.name.to_string())
        .collect();
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read golden dir") {
        let p = entry.expect("dir entry").path();
        if p.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if !names.contains(&stem) {
            orphans.push(stem);
        }
    }
    assert!(
        orphans.is_empty(),
        "goldens with no matching recipe: {orphans:?}"
    );
}

/// Every value a shipping recipe sets must satisfy the bound this project
/// invented for it.
///
/// The engine's clap definition carries value sets but no ranges at all, so
/// every `Int`/`Float` bound in the disposition table is atlasctl's own
/// judgement — and judgement drifts from the corpus silently. It did: the first
/// bound written for `request_timeout` was `1..=86400`, while
/// `qwen3.8-27b-nvfp4-throughput` ships `request_timeout: 0`, which the engine
/// documents as "disables the deadline entirely". Nothing would have failed
/// until an operator opened that recipe in the web form and was told its own
/// value was out of range.
#[test]
fn every_shipped_recipe_value_satisfies_its_own_bound() {
    let mut bad = Vec::new();

    for e in atlas_recipes_data::all() {
        let prov = Provenance::Builtin {
            path: e.path.to_string(),
        };
        let Ok(recipe) = Recipe::parse(e.name, e.yaml, prov) else {
            continue;
        };
        if !recipe.is_launchable() {
            continue;
        }

        for (key, value) in &recipe.defaults {
            // Only exposed keys have a bound to satisfy. A denied key is
            // recipe-only by design, and an unclaimed one is the coverage
            // check's problem, not this one's.
            let Some(Disposition::Expose(spec)) = settings::dispositions()
                .find(|(k, _)| k == key)
                .map(|(_, d)| d)
            else {
                continue;
            };
            let wire = match value {
                ScalarValue::Bool(b) => SettingValue::Bool(*b),
                ScalarValue::Int(i) => SettingValue::Int(*i),
                ScalarValue::Float(f) => SettingValue::Float(*f),
                ScalarValue::Str(s) => SettingValue::Str(s.clone()),
            };
            if let Err(err) = spec.bound.to_bound().check(key, &wire) {
                bad.push(format!("  {}: {key} = {value:?} — {err:?}", recipe.name));
            }
        }
    }

    assert!(
        bad.is_empty(),
        "a shipping recipe sets a value the web form would reject:\n{}",
        bad.join("\n")
    );
}
