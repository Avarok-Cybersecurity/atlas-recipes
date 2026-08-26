// SPDX-License-Identifier: AGPL-3.0-only

//! The check that keeps this project honest about the engine's flags.
//!
//! Reads the vendored snapshot rather than a transcription of it, so the thing
//! under test is the same artifact a regeneration would replace.

use super::EXCLUDED;
use crate::flags::{ATLAS_FLAGS, FlagKind};
use crate::settings::{BoundSpec, Disposition, dispositions};
use serde_json::Value;
use std::collections::BTreeSet;

/// The engine's own account of `spark serve`, as vendored.
const SNAPSHOT: &str = include_str!("../../../../../vendor/serve-options.v1.json");

struct EngineFlag {
    key: String,
    flag: String,
    presence_only: bool,
    options: Vec<String>,
    aliases: BTreeSet<String>,
}

fn engine_flags() -> Vec<EngineFlag> {
    let v: Value = serde_json::from_str(SNAPSHOT).expect("the vendored snapshot is JSON");
    assert_eq!(
        v["schema_version"], 1,
        "the snapshot changed schema; read the engine's cli/manifest.rs before touching this"
    );
    v["flags"]
        .as_array()
        .expect("flags is an array")
        .iter()
        .map(|f| {
            let strs = |k: &str| {
                f[k].as_array()
                    .map(|a| {
                        a.iter()
                            .map(|s| s.as_str().unwrap().to_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            let mut aliases: BTreeSet<String> = strs("cli_aliases").into_iter().collect();
            aliases.extend(strs("recipe_aliases"));
            EngineFlag {
                key: f["key"].as_str().unwrap().to_owned(),
                flag: f["flag"].as_str().unwrap().to_owned(),
                presence_only: f["presence_only"].as_bool().unwrap(),
                options: strs("options"),
                aliases,
            }
        })
        .collect()
}

/// Does the flag table claim this engine flag, under any of its spellings?
fn claimed(e: &EngineFlag) -> Option<&'static crate::flags::FlagSpec> {
    ATLAS_FLAGS.iter().find(|s| {
        let bare = s.flag.trim_start_matches('-');
        bare == e.flag
            || s.key == e.key
            || e.aliases.contains(bare)
            || e.aliases.contains(&s.key.replace('_', "-"))
            || e.aliases.iter().any(|a| a.replace('-', "_") == s.key)
    })
}

#[test]
fn every_engine_flag_is_either_claimed_or_excluded_on_the_record() {
    // The failure this prevents is silent. Before the snapshot existed, nine
    // keys in shipping recipes were dropped without a word — including
    // `lm_head_dtype`, which a recipe comment calls a correctness pin. A new
    // engine flag must not be able to join them by simply appearing.
    let mut orphans = Vec::new();
    for e in engine_flags() {
        if claimed(&e).is_none() && super::excluded_reason(&e.key).is_none() {
            orphans.push(e.key);
        }
    }
    assert!(
        orphans.is_empty(),
        "these engine flags are neither claimed nor excluded: {orphans:?}\n\
         Add each to ATLAS_FLAGS, or to EXCLUDED with the reason it is not passed through."
    );
}

#[test]
fn nothing_is_both_claimed_and_excluded() {
    for e in engine_flags() {
        if let Some(spec) = claimed(&e) {
            assert!(
                super::excluded_reason(&e.key).is_none(),
                "`{}` is emitted as `{}` and also listed as excluded",
                e.key,
                spec.flag
            );
        }
    }
}

#[test]
fn no_exclusion_names_a_flag_the_engine_does_not_have() {
    // An exclusion for a flag that no longer exists is a stale reason that
    // reads as a live decision, which is worse than no note at all.
    let known: BTreeSet<String> = engine_flags().into_iter().map(|e| e.key).collect();
    for (key, _) in EXCLUDED {
        assert!(
            known.contains(*key),
            "EXCLUDED names `{key}`, which is not a flag in the snapshot"
        );
    }
}

#[test]
fn a_bare_toggle_here_is_a_bare_toggle_there() {
    // This is the distinction no amount of reading a recipe can recover:
    // `video_allow_ffmpeg: true` and `gdn_fused_norm: true` are written
    // identically and emit differently, and only the engine knows which is
    // which. Getting it backwards produces `--flag true` where the engine
    // expects `--flag`, which fails at parse time inside the container.
    for e in engine_flags() {
        let Some(spec) = claimed(&e) else { continue };
        let ours = spec.kind == FlagKind::BoolToggle;
        assert_eq!(
            ours,
            e.presence_only,
            "`{}` is {} here and {} in the engine",
            e.key,
            if ours {
                "a bare toggle"
            } else {
                "a value flag"
            },
            if e.presence_only {
                "a bare toggle"
            } else {
                "a value flag"
            }
        );
    }
}

#[test]
fn no_picker_offers_a_value_the_engine_would_reject() {
    // `scheduling_policy` offered "fcfs" for four releases. The engine accepts
    // only fifo and slai, so every launch that chose it died in the container
    // with a parse error, and nothing upstream of that could tell you why.
    let engine = engine_flags();
    for (key, d) in dispositions() {
        let Disposition::Expose(spec) = d else {
            continue;
        };
        let BoundSpec::Enum(ours) = &spec.bound else {
            continue;
        };
        let Some(e) = engine
            .iter()
            .find(|e| claimed(e).is_some_and(|s| s.key == *key))
        else {
            continue;
        };
        if e.options.is_empty() {
            continue;
        }
        for v in *ours {
            assert!(
                e.options.iter().any(|o| o == v),
                "`{key}` offers {v:?}, which the engine does not accept; it takes {:?}",
                e.options
            );
        }
    }
}
