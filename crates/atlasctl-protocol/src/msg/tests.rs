// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn id(s: &str) -> RecipeId {
    RecipeId::parse(s).expect("valid fixture id")
}

#[test]
fn client_messages_are_tagged_on_type_for_the_browser_to_narrow_on() {
    let json = serde_json::to_string(&ClientMsg::Status { id: 7 }).unwrap();
    assert_eq!(json, r#"{"type":"status","id":7}"#);
}

#[test]
fn every_client_message_round_trips() {
    let msgs = vec![
        ClientMsg::Hello {
            protocol_version: 1,
            token: "t".into(),
        },
        ClientMsg::ListRecipes { id: 1 },
        ClientMsg::Preview {
            id: 2,
            recipe: id("r"),
            settings: BTreeMap::new(),
        },
        ClientMsg::Launch {
            id: 3,
            recipe: id("r"),
            settings: BTreeMap::new(),
        },
        ClientMsg::Stop {
            id: 4,
            recipe: id("r"),
        },
        ClientMsg::Status { id: 5 },
    ];
    for m in msgs {
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ClientMsg>(&s).unwrap(), m, "{s}");
    }
}

#[test]
fn the_client_surface_contains_no_relay_or_raw_command_verb() {
    // The capability claim, asserted rather than assumed. If someone adds a
    // verb that forwards arbitrary content, this fails and forces the argument
    // to be had in review.
    let variants = [
        r#"{"type":"exec","command":"sh"}"#,
        r#"{"type":"forward","to":"peer","payload":{}}"#,
        r#"{"type":"raw","argv":["docker","run","--privileged","x"]}"#,
        r#"{"type":"proxy","url":"http://evil"}"#,
        r#"{"type":"run_command","cmd":"rm -rf /"}"#,
    ];
    for v in variants {
        assert!(
            serde_json::from_str::<ClientMsg>(v).is_err(),
            "{v} deserialized — the client surface grew a verb it should not have"
        );
    }
}

#[test]
fn an_unknown_message_type_is_refused_rather_than_ignored() {
    assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"nope"}"#).is_err());
}

#[test]
fn a_launch_naming_a_hostile_recipe_fails_at_the_parse_boundary() {
    // Before any handler runs: the id type does the refusing.
    for bad in ["../../etc/passwd", "-rm", "a b", "A"] {
        let json = format!(r#"{{"type":"launch","id":1,"recipe":"{bad}"}}"#);
        assert!(
            serde_json::from_str::<ClientMsg>(&json).is_err(),
            "{bad} must not parse"
        );
    }
}

#[test]
fn a_launch_may_carry_settings_but_only_as_typed_values() {
    let json = r#"{"type":"launch","id":1,"recipe":"qwen3.6-27b-fp8",
                   "settings":{"port":9000,"speculative":true,"kv_cache_dtype":"fp8"}}"#;
    let ClientMsg::Launch { settings, .. } = serde_json::from_str(json).unwrap() else {
        panic!("expected a launch");
    };
    assert_eq!(settings["port"], SettingValue::Int(9000));
    assert_eq!(settings["speculative"], SettingValue::Bool(true));
    assert_eq!(settings["kv_cache_dtype"], SettingValue::Str("fp8".into()));
}

#[test]
fn a_launch_without_settings_is_valid() {
    let json = r#"{"type":"launch","id":1,"recipe":"qwen3.6-27b-fp8"}"#;
    assert!(serde_json::from_str::<ClientMsg>(json).is_ok());
}

#[test]
fn errors_carry_a_machine_readable_code() {
    let e = AgentError::UnknownRecipe {
        recipe: "nope".into(),
    };
    let json = serde_json::to_string(&ServerMsg::Error {
        id: Some(3),
        error: e,
    })
    .unwrap();
    assert!(json.contains(r#""code":"unknown_recipe""#), "{json}");
}

#[test]
fn the_welcome_frame_carries_a_version_range_so_mismatch_is_reported_not_hung() {
    let json = serde_json::to_string(&ServerMsg::Welcome {
        protocol_min: 1,
        protocol_max: 1,
        agent_version: "0.1.0".into(),
    })
    .unwrap();
    assert!(json.contains("protocol_min"), "{json}");
    assert!(json.contains("protocol_max"), "{json}");
}

#[test]
fn server_messages_round_trip() {
    let msgs = vec![
        ServerMsg::Welcome {
            protocol_min: 1,
            protocol_max: 1,
            agent_version: "0.1.0".into(),
        },
        ServerMsg::Stopped {
            id: 1,
            recipe: id("r"),
        },
        ServerMsg::Started {
            id: 2,
            recipe: id("r"),
            container: "atlas-r".into(),
            endpoint: Some("http://localhost:8888/v1".into()),
        },
        ServerMsg::Error {
            id: None,
            error: AgentError::NotPaired,
        },
    ];
    for m in msgs {
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ServerMsg>(&s).unwrap(), m, "{s}");
    }
}
