// SPDX-License-Identifier: AGPL-3.0-only

use super::bench::*;
use super::bench_event::*;
use super::bench_node::*;
use crate::fleet::{DisplayName, Metric, NodeId};
use std::collections::BTreeMap;

fn node() -> NodeId {
    NodeId::parse(&"ab".repeat(32)).unwrap()
}

fn sha() -> Sha {
    Sha::parse(&"1a".repeat(20)).unwrap()
}

#[test]
fn every_request_round_trips_through_json() {
    let reqs = vec![
        BenchReq::NodeInfo,
        BenchReq::Submit {
            job_key: JobKey::parse("certify-1-a-decode").unwrap(),
            sha: sha(),
            gate: GateId::parse("decode-floor").unwrap(),
            params: BTreeMap::from([("osl".to_string(), "8".to_string())]),
            checkpoint: None,
            hardware: Some("gb10".into()),
            max_run_s: Some(3600),
            note: Some("dogfood".into()),
        },
        BenchReq::Status { job: None },
        BenchReq::Cancel {
            job: JobId::parse("jb-1-deadbeef").unwrap(),
        },
        BenchReq::Attach {
            job: JobId::parse("jb-1-deadbeef").unwrap(),
            from_seq: 17,
        },
        BenchReq::ArtifactList {
            job: JobId::parse("jb-1-deadbeef").unwrap(),
        },
        BenchReq::ArtifactChunk {
            job: JobId::parse("jb-1-deadbeef").unwrap(),
            name: "x.json".into(),
            offset: 0,
            len: MAX_CHUNK,
        },
    ];
    for r in reqs {
        let text = serde_json::to_string(&r).unwrap();
        let back: BenchReq = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r, "{text}");
    }
}

#[test]
fn every_reply_and_refusal_round_trips() {
    let job = JobId::parse("jb-1-deadbeef").unwrap();
    let reps = vec![
        BenchRep::Accepted {
            job: job.clone(),
            existing: true,
            state: JobState::Running,
            position: 0,
        },
        BenchRep::Cancelled {
            job: job.clone(),
            state: JobState::Done,
        },
        BenchRep::Chunk {
            job: job.clone(),
            name: "x.json".into(),
            offset: 4096,
            data_b64: "AAAA".into(),
            eof: true,
        },
        BenchRep::Refused {
            by: node(),
            refusal: BenchRefusal::UnknownGate {
                gate: "nope".into(),
                known: vec!["decode-floor".into()],
            },
        },
        BenchRep::Refused {
            by: node(),
            refusal: BenchRefusal::NotGranted {
                command: "atlasctl peer grant-bench abcd".into(),
            },
        },
    ];
    for r in reps {
        let text = serde_json::to_string(&r).unwrap();
        let back: BenchRep = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r, "{text}");
    }
    // Refusals carry a stable `code` tag a CLI can switch on.
    let text = serde_json::to_string(&BenchRefusal::Busy {
        what: "spark pid 1".into(),
        retry_after_s: 60,
    })
    .unwrap();
    assert!(text.contains("\"code\":\"busy\""), "{text}");
}

#[test]
fn events_round_trip_and_done_flattens_its_outcome() {
    let job = JobId::parse("jb-1-deadbeef").unwrap();
    let ev = BenchEvent {
        job,
        seq: 3,
        at_ms: 1_789_000_000_000,
        kind: EventKind::Done {
            outcome: Outcome::Completed {
                exit_code: 0,
                verdict: Some(Verdict {
                    kind: VerdictKind::Pass,
                    text: "median 26.0".into(),
                }),
                record: Some(".benchmarks/decode-floor/x.json".into()),
                signature: Some(".benchmarks/decode-floor/x.json.sig".into()),
            },
        },
    };
    let text = serde_json::to_string(&ev).unwrap();
    assert!(text.contains("\"kind\":\"done\""), "{text}");
    assert!(text.contains("\"outcome\":\"completed\""), "{text}");
    let back: BenchEvent = serde_json::from_str(&text).unwrap();
    assert_eq!(back, ev);
    assert!(matches!(back.kind, EventKind::Done { outcome } if outcome.passed()));
    // An Info verdict is not a pass.
    let info = Outcome::Completed {
        exit_code: 0,
        verdict: Some(Verdict {
            kind: VerdictKind::Info,
            text: "shard".into(),
        }),
        record: None,
        signature: None,
    };
    assert!(!info.passed());
}

#[test]
fn newtypes_refuse_what_they_must() {
    assert!(JobKey::parse("certify-1-node_a.decode").is_ok());
    assert_eq!(JobKey::parse(""), Err(BenchIdError::Empty));
    assert_eq!(JobKey::parse("-rm"), Err(BenchIdError::FlagShaped));
    assert!(matches!(
        JobKey::parse("a b"),
        Err(BenchIdError::Charset(' '))
    ));
    assert!(matches!(
        JobKey::parse(&"x".repeat(65)),
        Err(BenchIdError::Length(65, 64))
    ));
    assert!(GateId::parse("bfcl-subset-a").is_ok());
    assert!(GateId::parse("../x").is_err());
    assert!(Sha::parse(&"1a".repeat(20)).is_ok());
    assert_eq!(
        Sha::parse("1a0dc88a8c"),
        Err(BenchIdError::NotASha),
        "short forms are refused"
    );
    assert_eq!(
        Sha::parse(&"1A".repeat(20)),
        Err(BenchIdError::NotASha),
        "uppercase is refused"
    );
    assert_eq!(sha().short(), "1a1a1a1a1a");
    // Deserialization applies the same rule.
    assert!(serde_json::from_str::<Sha>("\"abc\"").is_err());
    assert!(serde_json::from_str::<JobKey>("\"a;b\"").is_err());
}

#[test]
fn params_are_bounded() {
    let ok = BTreeMap::from([("osl".to_string(), "8".to_string())]);
    assert!(check_params(&ok).is_ok());
    let many: BTreeMap<String, String> = (0..33).map(|i| (format!("k{i}"), "v".into())).collect();
    assert!(check_params(&many).unwrap_err().contains("33 params"));
    let bad_key = BTreeMap::from([("a b".to_string(), "v".to_string())]);
    assert!(check_params(&bad_key).is_err());
    let long = BTreeMap::from([("k".to_string(), "v".repeat(257))]);
    assert!(check_params(&long).unwrap_err().contains("257 bytes"));
    let ctrl = BTreeMap::from([("k".to_string(), "a\nb".to_string())]);
    assert!(check_params(&ctrl).unwrap_err().contains("control"));
}

/// The largest chunk reply and the largest log event must fit a 1 MiB peer
/// frame with room for the envelope.
#[test]
fn the_largest_frames_stay_under_the_peer_cap() {
    let job = JobId::parse("jb-1-deadbeef").unwrap();
    let data = vec![0xffu8; MAX_CHUNK as usize];
    let b64 = base64_len(data.len());
    let chunk = BenchRep::Chunk {
        job: job.clone(),
        name: "x".repeat(255),
        offset: u64::MAX,
        data_b64: "A".repeat(b64),
        eof: false,
    };
    let bytes = serde_json::to_vec(&chunk).unwrap().len();
    assert!(bytes < 1024 * 1024 - 4096, "chunk reply is {bytes} bytes");
    let log = BenchEvent {
        job,
        seq: u64::MAX,
        at_ms: u64::MAX,
        kind: EventKind::Log {
            stream: LogStream::Run,
            lines: vec!["x".repeat(MAX_LOG_LINE_BYTES); MAX_LOG_LINES_PER_EVENT],
        },
    };
    let bytes = serde_json::to_vec(&log).unwrap().len();
    assert!(bytes < 256 * 1024, "log event is {bytes} bytes");
}

fn base64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

#[test]
fn node_info_round_trips() {
    let info = BenchNodeInfo {
        node: node(),
        name: DisplayName::new("spark-43fa"),
        agent_version: "0.5.0".into(),
        peer_version_max: 3,
        bench_enabled: true,
        disabled_reason: None,
        thermal: Some(super::bench_node::HostThermal {
            chassis_temps_c: vec![65.0, 62.0],
            throttle_thermal: Some(false),
            sm_clock_max_mhz: Some(3003.0),
            mem_total_kb: Some(127_601_452),
        }),
        gpu: Some(GpuInfo {
            name: "NVIDIA GB10".into(),
            count: 1,
            driver_version: "580.65".into(),
            cuda_version: "13.0".into(),
            sm_clock_mhz: Metric::Reading { value: 2405.0 },
            sm_clock_healthy_mhz: Some(2400),
            temperature_c: Metric::Reading { value: 55.0 },
            memory_total_bytes: Metric::Reading { value: 1.3e11 },
            memory_used_frac: Metric::Reading { value: 0.06 },
            memory_is_unified: true,
        }),
        alerts: vec![],
        hardware_class: Some("gb10".into()),
        atlas_repo: Some(RepoInfo {
            path: "/workspace/atlas".into(),
            remote_name: "origin".into(),
            remote_url: "https://github.com/Avarok-Cybersecurity/atlas.git".into(),
            head_sha: Some(sha()),
            fetched_at_s: Some(1),
        }),
        atlas_home: Some("/workspace/.atlas-c128".into()),
        signer_fp: Some("a27dbc8ed2fc2a31".into()),
        signer_pubkey_hex: Some("00".repeat(32)),
        recipes_synced: true,
        built_shas: vec![BuiltSha {
            sha: sha(),
            binary_sha256: "ab".repeat(32),
            built_at_s: 1,
            bytes: 10,
        }],
        busy: false,
        busy_reason: None,
        running_job: None,
        queued: 0,
        queue_depth: 2,
        disk_free_bytes: Metric::Reading { value: 9e10 },
        min_free_disk_bytes: 2e10 as u64,
        min_free_fraction: 0.85,
        host_free_fraction: Metric::Reading { value: 0.94 },
        max_run_s: 10800,
    };
    let text = serde_json::to_string(&info).unwrap();
    let back: BenchNodeInfo = serde_json::from_str(&text).unwrap();
    assert_eq!(back, info);
}
