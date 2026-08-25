// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::scalar::ScalarValue;

fn cfg(pairs: &[(&str, ScalarValue)]) -> BTreeMap<String, ScalarValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

#[test]
fn table_matches_the_reference_shape() {
    assert_eq!(
        ATLAS_FLAGS.len(),
        48,
        "flag count drifted from the reference"
    );
    assert_eq!(
        ATLAS_FLAGS
            .iter()
            .filter(|s| s.kind == FlagKind::BoolToggle)
            .count(),
        8,
        "bool-toggle count drifted from the reference"
    );
}

#[test]
fn every_key_is_unique_and_every_flag_is_well_formed() {
    let mut keys = std::collections::BTreeSet::new();
    for spec in ATLAS_FLAGS.iter() {
        assert!(keys.insert(spec.key), "duplicate key {}", spec.key);
        assert!(
            spec.flag.starts_with("--"),
            "{} is not a long flag",
            spec.flag
        );
    }
}

#[test]
fn value_flags_emit_flag_then_value_in_table_order() {
    // `port` precedes `gpu_memory_utilization` in the table, so it must precede
    // it in the output even though the map is sorted the other way.
    let resolved = cfg(&[
        ("gpu_memory_utilization", ScalarValue::Float(0.88)),
        ("port", ScalarValue::Int(8888)),
    ]);
    let (argv, unmapped) = render(&resolved, &[]);
    assert_eq!(argv, ["--port", "8888", "--gpu-memory-utilization", "0.88"]);
    assert!(unmapped.is_empty());
}

#[test]
fn truthy_toggle_emits_a_bare_flag_and_falsy_emits_nothing() {
    let (on, _) = render(&cfg(&[("speculative", ScalarValue::Bool(true))]), &[]);
    assert_eq!(on, ["--speculative"]);

    let (off, _) = render(&cfg(&[("speculative", ScalarValue::Bool(false))]), &[]);
    assert!(off.is_empty(), "a falsy toggle must emit nothing at all");
}

#[test]
fn skip_suppresses_a_key_the_caller_supplies_itself() {
    // Worker ranks force `--port 0`, so the resolved port must not also appear.
    let resolved = cfg(&[
        ("port", ScalarValue::Int(8888)),
        ("ep_size", ScalarValue::Int(2)),
    ]);
    let (argv, unmapped) = render(&resolved, &["port"]);
    assert_eq!(argv, ["--ep-size", "2"]);
    assert!(
        unmapped.is_empty(),
        "a skipped key is claimed, not reported as unmapped"
    );
}

#[test]
fn unmapped_keys_are_reported_rather_than_dropped_in_silence() {
    // `lm_head_dtype` is real: it is in four shipping recipes and the reference
    // implementation drops it without a word. Regression-guard the reporting.
    let resolved = cfg(&[
        ("port", ScalarValue::Int(8888)),
        ("lm_head_dtype", ScalarValue::Str("bf16".into())),
    ]);
    let (argv, unmapped) = render(&resolved, &[]);
    assert_eq!(argv, ["--port", "8888"]);
    assert_eq!(
        unmapped,
        [UnmappedKey {
            key: "lm_head_dtype".into(),
            rendered: "bf16".into()
        }]
    );
}

#[test]
fn aliased_keys_collide_instead_of_emitting_the_flag_twice() {
    let both = cfg(&[
        ("max_num_batched_tokens", ScalarValue::Int(2048)),
        ("max_prefill_tokens", ScalarValue::Int(4096)),
    ]);
    let err = validate_no_alias_conflict(&both).expect_err("alias conflict must be rejected");
    assert_eq!(err.flag, "--max-prefill-tokens");

    // Either one alone is fine.
    let one = cfg(&[("max_prefill_tokens", ScalarValue::Int(4096))]);
    assert!(validate_no_alias_conflict(&one).is_ok());
}

#[test]
fn api_key_renders_the_auth_token_flag() {
    // The one key whose flag name does not track its key name.
    let (argv, _) = render(&cfg(&[("api_key", ScalarValue::Str("sk-x".into()))]), &[]);
    assert_eq!(argv, ["--auth-token", "sk-x"]);
}
