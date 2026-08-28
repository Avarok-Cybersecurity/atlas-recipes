// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::flags;
use atlasctl_protocol::settings::Bound;

fn req(pairs: &[(&str, SettingValue)]) -> BTreeMap<String, SettingValue> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

#[test]
fn the_partition_is_exhaustive_over_the_flag_table() {
    // If this fails, a flag was added without deciding whether a webpage may
    // set it. Failing the build is the point: neither answer is a safe default.
    for spec in flags::ATLAS_FLAGS.iter() {
        assert!(
            disposition(spec.key).is_some(),
            "flag `{}` has no disposition — decide whether a client may set it",
            spec.key
        );
    }
    for (key, _) in dispositions() {
        assert!(
            flags::lookup(key).is_some(),
            "`{key}` has a disposition but is not a flag"
        );
    }
    assert_eq!(dispositions().count(), flags::ATLAS_FLAGS.len());
}

#[test]
fn no_key_appears_twice() {
    let mut seen = std::collections::BTreeSet::new();
    for (key, _) in dispositions() {
        assert!(seen.insert(*key), "duplicate disposition for `{key}`");
    }
}

#[test]
fn bare_toggles_are_toggles_and_value_flags_are_not() {
    for (key, d) in dispositions() {
        let Disposition::Expose(spec) = d else {
            continue;
        };
        let is_toggle = matches!(spec.bound.to_bound(), Bound::Toggle);
        let flag_is_toggle = flags::lookup(key).map(|f| f.kind) == Some(FlagKind::BoolToggle);
        assert_eq!(
            is_toggle, flag_is_toggle,
            "`{key}` disagrees with the flag table about being a bare toggle"
        );
    }
}

#[test]
fn every_path_credential_and_weight_selector_is_denied() {
    // Spelled out rather than derived, so removing one from the deny list is a
    // visible change to this test rather than a quiet change to a filter.
    for key in [
        "model_from_path",
        "cache_dir",
        "draft_model",
        "high_speed_swap_dir",
        "api_key",
        "require_auth",
        "gpu_ordinal",
        "served_model_name",
        "host",
    ] {
        assert!(
            matches!(disposition(key), Some(Disposition::Deny(_))),
            "`{key}` must not be settable by a client"
        );
    }
}

#[test]
fn denied_keys_never_reach_the_schema_a_client_sees() {
    let schema = schema();
    for (key, d) in dispositions() {
        if matches!(d, Disposition::Deny(_)) {
            assert!(
                !schema.iter().any(|s| s.key == *key),
                "denied key `{key}` was advertised to clients"
            );
        }
    }
}

#[test]
fn a_denied_key_is_refused_by_name_with_its_reason() {
    let err = validate(&req(&[(
        "model_from_path",
        SettingValue::Str("/etc/shadow".into()),
    )]))
    .expect_err("must be refused");
    match &err[0] {
        SettingError::Denied { key, reason } => {
            assert_eq!(key, "model_from_path");
            assert!(!reason.is_empty(), "a refusal should say why");
        }
        other => panic!("wrong error: {other:?}"),
    }
}

#[test]
fn an_unknown_key_is_refused() {
    let err = validate(&req(&[("totally_made_up", SettingValue::Int(1))])).expect_err("refused");
    assert!(matches!(err[0], SettingError::UnknownKey { .. }));
}

#[test]
fn valid_settings_pass_through_typed() {
    let ok = validate(&req(&[
        ("port", SettingValue::Int(9000)),
        ("gpu_memory_utilization", SettingValue::Float(0.85)),
        ("kv_cache_dtype", SettingValue::Str("fp8".into())),
        ("speculative", SettingValue::Bool(true)),
    ]))
    .expect("all valid");
    assert_eq!(ok["port"], ScalarValue::Int(9000));
    assert_eq!(ok["gpu_memory_utilization"], ScalarValue::Float(0.85));
    assert_eq!(ok["kv_cache_dtype"], ScalarValue::Str("fp8".into()));
    assert_eq!(ok["speculative"], ScalarValue::Bool(true));
}

#[test]
fn every_problem_is_reported_at_once() {
    // Fixing a form one rejection at a time is a bad experience and encourages
    // people to stop reading the errors.
    let err = validate(&req(&[
        ("port", SettingValue::Int(1)),
        ("cache_dir", SettingValue::Str("/tmp".into())),
        ("nope", SettingValue::Int(1)),
    ]))
    .expect_err("must be refused");
    assert_eq!(err.len(), 3, "got {err:?}");
}

#[test]
fn nothing_a_client_sends_can_become_a_flag_shaped_argv_element() {
    // The end-to-end structural claim, exercised across every exposed setting.
    let hostile = [
        SettingValue::Str("--model-from-path /etc/shadow".into()),
        SettingValue::Str("; rm -rf /".into()),
        SettingValue::Str("$(id)".into()),
        SettingValue::Int(i64::MAX),
        SettingValue::Float(f64::NAN),
    ];
    for (key, d) in dispositions() {
        if !matches!(d, Disposition::Expose(_)) {
            continue;
        }
        for v in &hostile {
            if let Ok(ok) = validate(&req(&[(key, v.clone())])) {
                let rendered = ok[*key].render();
                assert!(
                    !rendered.starts_with('-'),
                    "`{key}` accepted {v:?} and rendered {rendered:?}"
                );
                assert!(
                    !rendered.contains(|c: char| c.is_whitespace() || ";|&$`".contains(c)),
                    "`{key}` accepted {v:?} and rendered {rendered:?}"
                );
            }
        }
    }
}

/// These two lists are transcribed from the engine, which is a different repo
/// and cannot be renamed atomically with this one. The counts are recorded so
/// a drift fails here — cheaply, in CI — rather than as a launch that dies
/// inside the container after the operator has reviewed the command.
///
/// If one of these fails, check the engine's `cli::flag_values` and
/// `spark_runtime::kv_cache::KvCacheDtype::ALL`, then update the table.
#[test]
fn the_enumerated_values_still_match_the_engine() {
    let kv = super::dispositions()
        .find(|(k, _)| *k == "kv_cache_dtype")
        .and_then(|(_, d)| match d {
            super::spec::Disposition::Expose(sp) => Some(sp),
            super::spec::Disposition::Deny(_) => None,
        })
        .expect("kv_cache_dtype is exposed");
    let super::spec::BoundSpec::Enum(kvs) = &kv.bound else {
        panic!("kv_cache_dtype must be an enum");
    };
    assert_eq!(
        kvs.len(),
        16,
        "KvCacheDtype::ALL has 16 variants; this offered {}",
        kvs.len()
    );

    let sp = super::dispositions()
        .find(|(k, _)| *k == "scheduling_policy")
        .and_then(|(_, d)| match d {
            super::spec::Disposition::Expose(sp) => Some(sp),
            super::spec::Disposition::Deny(_) => None,
        })
        .expect("scheduling_policy is exposed");
    let super::spec::BoundSpec::Enum(policies) = &sp.bound else {
        panic!("scheduling_policy must be an enum");
    };
    assert_eq!(
        *policies,
        ["fifo", "slai"].as_slice(),
        "the engine accepts fifo and slai; \"fcfs\" was offered for four releases \
         and killed every launch that chose it"
    );
}

// ---- CLI overrides against declared bounds ---------------------------------
//
// `validate` serves a webpage and refuses denied and unknown keys too.
// `check_override` serves a local operator and must NOT: the deny list says
// what a remote client may set, and an unknown key already draws a warning
// naming the value it will not apply. What it must catch is the case the CLI
// passed through in silence — a value outside the range the project declares.

#[test]
fn a_value_outside_the_declared_range_is_refused() {
    // Rendered `--gpu-memory-utilization 5` before this, against a declared
    // Float(0.10, 0.95), on hardware where 0.90 has needed a power cycle.
    let e = super::check_override("gpu_memory_utilization", &ScalarValue::Float(5.0))
        .expect_err("5 is not a fraction");
    assert!(e.to_string().contains("0.95"), "names the ceiling: {e}");

    let e =
        super::check_override("port", &ScalarValue::Int(99_999)).expect_err("above the port range");
    assert!(e.to_string().contains("49151"), "{e}");
}

#[test]
fn values_inside_the_range_pass_including_the_endpoints() {
    for v in [0.10_f64, 0.85, 0.95] {
        super::check_override("gpu_memory_utilization", &ScalarValue::Float(v))
            .unwrap_or_else(|e| panic!("{v} is inside the declared bound: {e}"));
    }
    super::check_override("port", &ScalarValue::Int(1024)).expect("lower endpoint");
    super::check_override("port", &ScalarValue::Int(49_151)).expect("upper endpoint");
}

/// A local shell is not a webpage. Refusing these here would stop someone
/// configuring their own machine, which is not what the deny list is for.
#[test]
fn a_denied_or_unknown_key_is_not_this_functions_business() {
    super::check_override("host", &ScalarValue::Str("0.0.0.0".into()))
        .expect("denied keys are a remote-client rule, not a local one");
    super::check_override("nosuchkey", &ScalarValue::Int(1))
        .expect("unknown keys are warned about by the launcher, not refused here");
}
