// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn int(min: i64, max: i64) -> Bound {
    Bound::Int { min, max }
}

#[test]
fn integers_are_accepted_only_inside_their_range() {
    let b = int(1024, 49151);
    assert!(b.check("port", &SettingValue::Int(8888)).is_ok());
    assert!(
        b.check("port", &SettingValue::Int(1024)).is_ok(),
        "min is inclusive"
    );
    assert!(
        b.check("port", &SettingValue::Int(49151)).is_ok(),
        "max is inclusive"
    );
    assert!(b.check("port", &SettingValue::Int(1023)).is_err());
    assert!(b.check("port", &SettingValue::Int(49152)).is_err());
    assert!(b.check("port", &SettingValue::Int(-1)).is_err());
}

#[test]
fn floats_reject_nan_and_infinity() {
    // A NaN comparison is always false, so a naive range check would let it
    // through and it would then render as "NaN" on a command line.
    let b = Bound::Float {
        min: 0.1,
        max: 0.95,
    };
    assert!(b.check("g", &SettingValue::Float(f64::NAN)).is_err());
    assert!(b.check("g", &SettingValue::Float(f64::INFINITY)).is_err());
    assert!(
        b.check("g", &SettingValue::Float(f64::NEG_INFINITY))
            .is_err()
    );
    assert!(b.check("g", &SettingValue::Float(0.88)).is_ok());
}

#[test]
fn a_whole_number_is_an_acceptable_float() {
    let b = Bound::Float { min: 0.0, max: 1.0 };
    assert_eq!(
        b.check("g", &SettingValue::Int(1)).unwrap(),
        SettingValue::Float(1.0)
    );
}

#[test]
fn enums_accept_only_their_own_variants() {
    let b = Bound::Enum {
        variants: vec!["bf16".into(), "fp8".into(), "nvfp4".into()],
    };
    assert!(b.check("kv", &SettingValue::Str("fp8".into())).is_ok());
    assert!(
        b.check("kv", &SettingValue::Str("FP8".into())).is_err(),
        "case must match exactly"
    );
    assert!(
        b.check("kv", &SettingValue::Str("fp8 --evil".into()))
            .is_err()
    );
    assert!(b.check("kv", &SettingValue::Int(8)).is_err());
}

#[test]
fn an_accepted_enum_value_is_the_stored_variant_not_the_submitted_string() {
    // Load-bearing: the value that continues into the command comes from our
    // table, so client bytes never reach an argv element even when they match.
    let b = Bound::Enum {
        variants: vec!["fp8".into()],
    };
    let out = b
        .check("kv", &SettingValue::Str(String::from("fp8")))
        .unwrap();
    assert_eq!(out, SettingValue::Str("fp8".into()));
}

#[test]
fn no_bound_can_hold_an_arbitrary_string() {
    // The structural defence. If a free-string kind is ever added, this fails
    // and forces the decision to be made deliberately.
    let hostile = SettingValue::Str("--model-from-path /etc/shadow".into());
    for b in [
        int(0, 10),
        Bound::Float { min: 0.0, max: 1.0 },
        Bound::Enum {
            variants: vec!["a".into()],
        },
        Bound::Toggle,
        Bound::BoolValue,
        Bound::IntOrAuto { min: 0, max: 10 },
    ] {
        assert!(
            b.check("k", &hostile).is_err(),
            "{b:?} accepted an arbitrary string"
        );
    }
}

#[test]
fn int_or_auto_accepts_the_literal_auto_and_nothing_else_textual() {
    let b = Bound::IntOrAuto { min: 0, max: 128 };
    assert!(b.check("k", &SettingValue::Str("auto".into())).is_ok());
    assert!(b.check("k", &SettingValue::Int(16)).is_ok());
    assert!(b.check("k", &SettingValue::Int(129)).is_err());
    assert!(
        b.check("k", &SettingValue::Str("automatic".into()))
            .is_err()
    );
    assert!(
        b.check("k", &SettingValue::Str("auto; rm -rf /".into()))
            .is_err()
    );
}

#[test]
fn toggles_take_booleans_only() {
    assert!(Bound::Toggle.check("k", &SettingValue::Bool(true)).is_ok());
    // "true" as a string is a different thing and must not be coerced.
    assert!(
        Bound::Toggle
            .check("k", &SettingValue::Str("true".into()))
            .is_err()
    );
    assert!(Bound::Toggle.check("k", &SettingValue::Int(1)).is_err());
}

#[test]
fn errors_serialize_with_a_machine_readable_code() {
    // The browser switches on these rather than matching error prose.
    let e = SettingError::OutOfRange {
        key: "port".into(),
        min: "1024".into(),
        max: "49151".into(),
    };
    let json = serde_json::to_string(&e).unwrap();
    assert!(json.contains(r#""code":"out_of_range""#), "{json}");
    assert!(json.contains(r#""key":"port""#), "{json}");
}

#[test]
fn a_spec_round_trips_through_json() {
    let spec = SettingSpec {
        key: "port".into(),
        bound: int(1024, 49151),
        label: "Port".into(),
        help: "Port the model server listens on.".into(),
        unit: None,
        group: Group::Server,
        advanced: false,
        locked: false,
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(serde_json::from_str::<SettingSpec>(&json).unwrap(), spec);
}
