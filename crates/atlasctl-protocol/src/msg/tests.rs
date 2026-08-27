// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn id(s: &str) -> RecipeId {
    RecipeId::parse(s).expect("valid fixture id")
}

#[test]
fn client_messages_are_tagged_on_type_for_the_browser_to_narrow_on() {
    let json = serde_json::to_string(&ClientMsg::Status { id: 7, on: None }).unwrap();
    // `on` serialises as an explicit null rather than being omitted: an
    // absent value is stated, never guessed at by the reader.
    assert_eq!(json, r#"{"type":"status","id":7,"on":null}"#);
}

#[test]
fn every_client_message_round_trips() {
    let msgs = vec![
        ClientMsg::Hello {
            protocol_version: crate::PROTOCOL_VERSION,
            token: "t".into(),
        },
        ClientMsg::ListRecipes { id: 1, on: None },
        ClientMsg::Preview {
            id: 2,
            recipe: id("r"),
            settings: BTreeMap::new(),
            on: None,
        },
        ClientMsg::Launch {
            id: 3,
            recipe: id("r"),
            settings: BTreeMap::new(),
            on: None,
        },
        ClientMsg::Stop {
            id: 4,
            recipe: id("r"),
            on: None,
        },
        ClientMsg::Status { id: 5, on: None },
        // The annotated form must survive the wire too, or forwarding would
        // silently degrade to local execution.
        ClientMsg::Status {
            id: 6,
            on: Some(crate::fleet::NodeId::from_bytes([7; 32])),
        },
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
            on: None,
            via: None,
        },
        ServerMsg::Started {
            id: 2,
            recipe: id("r"),
            container: "atlas-r".into(),
            endpoint: Some("http://localhost:8888/v1".into()),
            // The forwarded shape: dgx2's answer, carried by dgx1. Round-
            // tripping it proves provenance survives the wire.
            on: Some(crate::fleet::NodeId::from_bytes([2; 32])),
            via: Some(crate::fleet::NodeId::from_bytes([1; 32])),
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

// ---- fleet verbs ---------------------------------------------------------

#[test]
fn the_fleet_verbs_did_not_open_a_relay() {
    // The capability claim, re-asserted now that the surface has grown. The
    // fleet verbs let a page name a peer it has already paired and ask for a
    // launch; they must not let it forward arbitrary content to that peer.
    let must_not_parse = [
        r#"{"type":"forward","to":"peer","payload":{}}"#,
        r#"{"type":"relay","node":"aa","frame":{}}"#,
        r#"{"type":"peer_exec","node":"aa","command":"sh"}"#,
        r#"{"type":"peer_raw","node":"aa","argv":["docker","run"]}"#,
        r#"{"type":"proxy","url":"http://evil"}"#,
    ];
    for v in must_not_parse {
        assert!(
            serde_json::from_str::<ClientMsg>(v).is_err(),
            "{v} deserialized — the fleet verbs grew a relay"
        );
    }
}

#[test]
fn a_cluster_launch_names_nodes_and_a_recipe_and_nothing_executable() {
    // A page may say "run this recipe on these two paired nodes". It may not
    // say what command to run, which image to pull, or what environment to set.
    let msg = ClientMsg::PrepareCluster {
        id: 7,
        recipe: id("qwen3.5-122b-a10b-nvfp4-ep2"),
        nodes: vec![
            crate::fleet::NodeId::from_bytes([1; 32]),
            crate::fleet::NodeId::from_bytes([2; 32]),
        ],
        head: crate::fleet::NodeId::from_bytes([1; 32]),
        settings: BTreeMap::new(),
    };
    let json = serde_json::to_string(&msg).expect("serialises");
    for forbidden in ["image", "argv", "command", "entrypoint", "env", "volume"] {
        assert!(
            !json.contains(forbidden),
            "cluster launch carries `{forbidden}`"
        );
    }
    assert_eq!(
        serde_json::from_str::<ClientMsg>(&json).expect("round trips"),
        msg
    );
}

#[test]
fn a_commit_is_pinned_to_the_prepare_that_produced_it() {
    // Without the epoch, a commit could be replayed against a plan that has
    // since changed — a different recipe, or a different set of nodes.
    let m = ClientMsg::CommitCluster {
        id: 1,
        epoch: "e-123".to_owned(),
    };
    let json = serde_json::to_string(&m).expect("serialises");
    assert!(json.contains("e-123"));
    // A commit with no epoch must not parse.
    assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"commit_cluster","id":1}"#).is_err());
}

#[test]
fn a_pairing_request_carries_a_code_but_never_produces_one() {
    // The browser transcribes a code read off the target machine. There is no
    // verb that asks this agent to mint a code for a page, because that would
    // let a page pair a machine on its own.
    assert!(
        serde_json::from_str::<ClientMsg>(r#"{"type":"issue_pair_code","id":1}"#).is_err(),
        "a page must never be able to mint a pairing code"
    );
    let m = ClientMsg::PairPeer {
        id: 2,
        node: crate::fleet::NodeId::from_bytes([9; 32]),
        code: "13572468".to_owned(),
    };
    assert_eq!(
        serde_json::from_str::<ClientMsg>(&serde_json::to_string(&m).expect("ser")).expect("de"),
        m
    );
}

#[test]
fn fleet_events_round_trip_including_the_absent_metric_state() {
    use crate::fleet::{Metric, NodeVitals};
    use crate::msg::fleet::FleetEvent;

    let ev = FleetEvent::Vitals {
        node: crate::fleet::NodeId::from_bytes([3; 32]),
        vitals: Box::new(NodeVitals {
            accelerator_util: Metric::reading(96.0),
            // The GB10 case has to survive the wire as "cannot answer", not 0.
            memory_total_bytes: Metric::Unsupported,
            ..NodeVitals::default()
        }),
    };
    let json = serde_json::to_string(&ev).expect("serialises");
    assert!(json.contains(r#""change":"vitals""#));
    assert!(json.contains("unsupported"));
    let back: FleetEvent = serde_json::from_str(&json).expect("round trips");
    assert_eq!(back, ev);
}

// ---- protocol 4: forwarding annotations and the control vocabulary -------

/// The `on` annotation of a single-node control verb, or `None` for anything
/// else. A helper rather than duplicated matches, so the seven-verb set is
/// written down exactly once in these tests.
fn on_of(m: &ClientMsg) -> Option<&Option<crate::fleet::NodeId>> {
    match m {
        ClientMsg::ListRecipes { on, .. }
        | ClientMsg::Preview { on, .. }
        | ClientMsg::Launch { on, .. }
        | ClientMsg::Stop { on, .. }
        | ClientMsg::Status { on, .. }
        | ClientMsg::LaunchStats { on, .. }
        | ClientMsg::LaunchLogs { on, .. } => Some(on),
        _ => None,
    }
}

#[test]
fn a_protocol_3_request_without_on_still_decodes_and_means_this_machine() {
    // The wire a protocol-3 page produced. Refusing it — or worse, guessing a
    // target — would break every deployed client for a field it never knew.
    let v3 = [
        r#"{"type":"list_recipes","id":1}"#,
        r#"{"type":"preview","id":2,"recipe":"r"}"#,
        r#"{"type":"launch","id":3,"recipe":"r"}"#,
        r#"{"type":"stop","id":4,"recipe":"r"}"#,
        r#"{"type":"status","id":5}"#,
        r#"{"type":"launch_stats","id":6,"recipe":"r"}"#,
        r#"{"type":"launch_logs","id":7,"recipe":"r","lines":50}"#,
    ];
    for raw in v3 {
        let m: ClientMsg = serde_json::from_str(raw).expect(raw);
        assert_eq!(on_of(&m), Some(&None), "{raw} must decode with on == None");
    }
}

#[test]
fn a_protocol_3_pairing_request_decodes_with_no_control_grant() {
    // Consent to remote control must be said, not implied by upgrading: the
    // old wire shape grants nothing.
    let m: ClientMsg = serde_json::from_str(&format!(
        r#"{{"type":"confirm_pairing","id":1,"node":"{FP}"}}"#
    ))
    .expect("v3 confirm_pairing decodes");
    assert!(matches!(
        m,
        ClientMsg::ConfirmPairing {
            allow_control: false,
            ..
        }
    ));
    let m: ClientMsg =
        serde_json::from_str(r#"{"type":"mint_join_code","id":2}"#).expect("v3 mint decodes");
    assert!(matches!(
        m,
        ClientMsg::MintJoinCode {
            allow_control: false,
            ..
        }
    ));
}

#[test]
fn a_protocol_3_reply_decodes_with_no_provenance() {
    // The CLI and tests replay stored replies; a recipes reply written before
    // `on`/`via` existed is a local, direct answer and must read as one.
    let m: ServerMsg = serde_json::from_str(r#"{"type":"recipes","id":1,"recipes":[]}"#)
        .expect("v3 recipes reply decodes");
    assert!(matches!(
        m,
        ServerMsg::Recipes {
            on: None,
            via: None,
            ..
        }
    ));
    let m: ServerMsg = serde_json::from_str(r#"{"type":"stopped","id":2,"recipe":"r"}"#)
        .expect("v3 stopped reply decodes");
    assert!(matches!(
        m,
        ServerMsg::Stopped {
            on: None,
            via: None,
            ..
        }
    ));
}

const FP: &str = "3f2a1b0c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8";

fn peer() -> crate::fleet::NodeId {
    crate::fleet::NodeId::parse(FP).expect("fixture is a valid fingerprint")
}

#[test]
fn control_requests_round_trip() {
    let reqs = vec![
        ControlReq::ListRecipes,
        ControlReq::Preview {
            recipe: id("r"),
            settings: BTreeMap::new(),
        },
        ControlReq::Launch {
            recipe: id("r"),
            settings: BTreeMap::new(),
        },
        ControlReq::Stop { recipe: id("r") },
        ControlReq::Status,
        ControlReq::Stats { recipe: id("r") },
        ControlReq::Logs {
            recipe: id("r"),
            lines: 100,
        },
    ];
    for r in reqs {
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ControlReq>(&s).unwrap(), r, "{s}");
    }
}

#[test]
fn control_replies_round_trip_including_a_named_refusal() {
    let reps = vec![
        ControlRep::Recipes {
            recipes: Vec::new(),
        },
        ControlRep::Started {
            recipe: id("r"),
            container: "atlas-r".into(),
            endpoint: Some("http://localhost:8888/v1".into()),
        },
        ControlRep::Refused {
            by: peer(),
            error: AgentError::AlreadyRunning { recipe: id("r") },
        },
    ];
    for r in reps {
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ControlRep>(&s).unwrap(), r, "{s}");
    }
}

#[test]
fn a_control_request_cannot_express_pairing_forwarding_or_the_cluster() {
    // The safety property of the whole design: `ControlReq` is a closed enum
    // that is NOT `ClientMsg`, so "never carries pairing frames" and "one
    // hop, never nested" hold because there is no variant a filter could
    // miss. If any of these ever decodes, the relay has become an open proxy.
    let must_not_parse = [
        // Pairing-shaped.
        format!(r#"{{"type":"pair_peer","node":"{FP}","code":"12345678"}}"#),
        format!(r#"{{"type":"confirm_pairing","node":"{FP}"}}"#),
        r#"{"type":"mint_join_code"}"#.to_owned(),
        r#"{"type":"hello","protocol_version":4,"token":"t"}"#.to_owned(),
        // Forward-shaped: a nested hop must be unrepresentable.
        format!(r#"{{"type":"control_to","node":"{FP}","req":{{"type":"status"}}}}"#),
        r#"{"type":"forward","to":"peer","payload":{"type":"status"}}"#.to_owned(),
        r#"{"type":"relay","node":"aa","frame":{}}"#.to_owned(),
        // Cluster-shaped.
        format!(r#"{{"type":"prepare_cluster","recipe":"r","nodes":["{FP}"],"head":"{FP}"}}"#),
        r#"{"type":"stop_cluster"}"#.to_owned(),
        // Raw-command-shaped.
        r#"{"type":"exec","command":"sh"}"#.to_owned(),
    ];
    for raw in must_not_parse {
        assert!(
            serde_json::from_str::<ControlReq>(&raw).is_err(),
            "{raw} deserialized — the forwardable vocabulary grew a verb it must never have"
        );
    }
}

#[test]
fn the_routing_errors_carry_machine_readable_codes_and_round_trip() {
    let errors = [
        AgentError::NotRoutable {
            node: peer(),
            reason: "not paired with dgx2, and dgx1, which vouches for it, is unreachable".into(),
        },
        AgentError::RelayRefused {
            node: peer(),
            detail: "requester lacks the controller grant on this machine".into(),
        },
        AgentError::ControlRefused {
            node: peer(),
            reason: "run `atlasctl peer grant-control <sender>` on that machine".into(),
        },
    ];
    let codes = ["not_routable", "relay_refused", "control_refused"];
    for (e, code) in errors.into_iter().zip(codes) {
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(&format!(r#""code":"{code}""#)), "{s}");
        assert_eq!(serde_json::from_str::<AgentError>(&s).unwrap(), e, "{s}");
    }
}
