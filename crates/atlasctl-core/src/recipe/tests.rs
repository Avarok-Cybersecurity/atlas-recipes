// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn builtin() -> Provenance {
    Provenance::Builtin {
        path: "fixture/r.yaml".into(),
    }
}

fn parse(yaml: &str) -> Result<Recipe, RecipeError> {
    Recipe::parse("r", yaml, builtin())
}

const MINIMAL: &str = "model: org/m\ncontainer: img:tag\nruntime: atlas\n";

#[test]
fn a_minimal_recipe_parses_and_is_launchable() {
    let r = parse(MINIMAL).expect("parses");
    assert_eq!(r.model, "org/m");
    assert_eq!(r.container, "img:tag");
    assert_eq!(r.runtime, RuntimeKind::Atlas);
    assert!(r.is_launchable());
    assert_eq!(r.qualified_name(), "@atlas/r");
    // Absent recipe_version defaults to the current schema.
    assert_eq!(r.recipe_version, "2");
}

#[test]
fn a_recipe_without_a_model_is_rejected() {
    let err = parse("container: img:tag\n").expect_err("must be rejected");
    assert!(
        matches!(err, RecipeError::MissingModel { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_recipe_without_a_container_is_rejected() {
    let err = parse("model: org/m\n").expect_err("must be rejected");
    assert!(
        matches!(err, RecipeError::MissingContainer { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_blank_model_is_as_absent_as_a_missing_one() {
    let err = parse("model: '   '\ncontainer: i\n").expect_err("must be rejected");
    assert!(
        matches!(err, RecipeError::MissingModel { .. }),
        "got {err:?}"
    );
}

#[test]
fn every_executable_content_key_blocks_launching() {
    // The security boundary: each of these ran code in the reference tool. The
    // recipe still loads (so it can be listed and explained) but never runs.
    for key in EXECUTABLE_KEYS.iter().chain(ISOLATION_KEYS.iter()) {
        let yaml = format!("{MINIMAL}{key}: []\n");
        let r = parse(&yaml).expect("must still load, so `recipe show` can explain it");
        match r.launchable() {
            Err(NotLaunchable::ExecutableContent(keys)) => {
                assert!(
                    keys.contains(&(*key).to_string()),
                    "{key} not named in {keys:?}"
                );
            }
            other => panic!("{key} must block launching, got {other:?}"),
        }
    }
}

#[test]
fn refusal_names_every_offending_key_at_once() {
    let yaml = format!("{MINIMAL}pre_exec: []\npost_commands: []\n");
    let r = parse(&yaml).expect("loads");
    match r.launchable() {
        Err(NotLaunchable::ExecutableContent(keys)) => {
            assert!(keys.contains(&"pre_exec".to_string()));
            assert!(keys.contains(&"post_commands".to_string()));
        }
        other => panic!("wrong outcome: {other:?}"),
    }
}

#[test]
fn unknown_keys_are_reported_but_do_not_block_loading() {
    // Two real examples from a dev recipe in the corpus.
    let yaml = format!("{MINIMAL}reference:\n  engine: vllm\nhealth:\n  probe_when: [before]\n");
    let r = parse(&yaml).expect("unknown keys must not be fatal");
    assert!(r.unknown_keys.contains(&"reference".to_string()));
    assert!(r.unknown_keys.contains(&"health".to_string()));
}

#[test]
fn a_non_atlas_runtime_loads_but_is_not_launchable() {
    let yaml = "model: org/m\ncontainer: i\nruntime: vllm-distributed\n";
    let r = parse(yaml).expect("must still load, so it can be listed and explained");
    assert_eq!(
        r.runtime,
        RuntimeKind::Unsupported("vllm-distributed".into())
    );
    assert_eq!(
        r.launchable(),
        Err(NotLaunchable::ForeignRuntime("vllm-distributed".into()))
    );
}

#[test]
fn a_legacy_recipe_with_no_runtime_is_not_launchable() {
    // The reference implementation guessed the runtime by sniffing the command
    // template. Guessing what to execute is exactly what we do not do.
    let r = parse("model: org/m\ncontainer: i\nrecipe_version: '1'\n").expect("loads");
    assert_eq!(r.runtime, RuntimeKind::Unsupported(String::new()));
    assert_eq!(
        r.launchable(),
        Err(NotLaunchable::NoRuntimeDeclared("1".into()))
    );
}

#[test]
fn description_falls_back_to_metadata() {
    let yaml = format!("{MINIMAL}metadata:\n  description: from metadata\n");
    assert_eq!(
        parse(&yaml).unwrap().description.as_deref(),
        Some("from metadata")
    );

    let yaml = format!("{MINIMAL}description: top level\nmetadata:\n  description: ignored\n");
    assert_eq!(
        parse(&yaml).unwrap().description.as_deref(),
        Some("top level")
    );
}

#[test]
fn defaults_preserve_their_yaml_scalar_types() {
    let yaml = format!(
        "{MINIMAL}defaults:\n  port: 8888\n  gpu_memory_utilization: 0.88\n  \
         speculative: true\n  kv_cache_dtype: fp8\n"
    );
    let d = parse(&yaml).unwrap().defaults;
    assert_eq!(d["port"], ScalarValue::Int(8888));
    assert_eq!(d["gpu_memory_utilization"], ScalarValue::Float(0.88));
    assert_eq!(d["speculative"], ScalarValue::Bool(true));
    assert_eq!(d["kv_cache_dtype"], ScalarValue::Str("fp8".into()));
}

#[test]
fn qualified_name_reflects_provenance() {
    let remote = Recipe::parse(
        "r",
        MINIMAL,
        Provenance::Remote {
            registry: "third".into(),
            url: "https://x/y.git".into(),
        },
    )
    .unwrap();
    assert_eq!(remote.qualified_name(), "@third/r");
}

/// `model` lands in `spark serve <model>` and `container` in the image operand
/// of `docker run`, both positionally and neither behind a `--`. A value
/// starting with `-` is handed straight to an option parser — `container:
/// --privileged` makes docker read a flag and take the NEXT token as the
/// image, which is an untrusted field steering the runtime rather than a
/// launch that cleanly fails.
///
/// The audited guarantee that no override VALUE can become flag-shaped covers
/// what a client sends; these two are recipe fields and were the gap.
#[test]
fn a_recipe_field_that_would_be_read_as_an_option_is_refused() {
    for (field, yaml) in [
        ("container", "model: org/m\ncontainer: \"--privileged\"\n"),
        ("model", "model: \"--help\"\ncontainer: img\n"),
    ] {
        let err = parse(yaml).expect_err("must refuse a flag-shaped field");
        let text = format!("{err}");
        assert!(text.contains(field), "must name the field: {text}");
        assert!(
            text.contains("option"),
            "must say why it is refused: {text}"
        );
    }
}

/// A leading dash is the hazard; a dash anywhere else is an ordinary character
/// in an image tag or a model id, and refusing those would break real recipes.
#[test]
fn an_interior_dash_is_still_an_ordinary_character() {
    let r = parse("model: unsloth/Qwen3.8-27B-NVFP4\ncontainer: avarok/atlas-gb10:latest\n")
        .expect("a normal recipe still parses");
    assert_eq!(r.model, "unsloth/Qwen3.8-27B-NVFP4");
}
