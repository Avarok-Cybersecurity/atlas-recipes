// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_protocol::fleet::{AlertKind, DisplayName, Metric, NodeAlert, NodeId, Severity};
use atlasctl_protocol::msg::bench::{GateId, JobId, JobKey, JobState, Sha};
use atlasctl_protocol::msg::bench_event::{
    ArtifactKind, ArtifactMeta, Outcome, Verdict, VerdictKind,
};
use atlasctl_protocol::msg::bench_node::{BuiltSha, GpuInfo, RepoInfo};

const SHA: &str = "1a0dc88a8c9083bb956bd84cafa2cccbdb8e6e18";

fn summary(state: JobState, outcome: Option<Outcome>) -> JobSummary {
    JobSummary {
        job: JobId::parse("jb-1757770000-deadbeef").unwrap(),
        job_key: JobKey::parse("k").unwrap(),
        sha: Sha::parse(SHA).unwrap(),
        gate: GateId::parse("decode-floor").unwrap(),
        state,
        created_at_s: 0,
        updated_at_s: 0,
        seq_high: 42,
        outcome,
    }
}

fn info() -> BenchNodeInfo {
    BenchNodeInfo {
        node: NodeId::from_bytes([9; 32]),
        name: DisplayName::new("spark-43fa"),
        agent_version: "0.5.0".into(),
        peer_version_max: 3,
        bench_enabled: true,
        disabled_reason: None,
        gpu: Some(GpuInfo {
            name: "NVIDIA GB10".into(),
            count: 1,
            driver_version: "580.95".into(),
            cuda_version: String::new(),
            sm_clock_mhz: Metric::reading(1500.0),
            sm_clock_healthy_mhz: Some(1500),
            temperature_c: Metric::Unsupported,
            memory_total_bytes: Metric::reading(128.0 * 1024.0 * 1024.0 * 1024.0),
            memory_used_frac: Metric::reading(0.1),
            memory_is_unified: true,
        }),
        thermal: Some(atlasctl_protocol::msg::bench_node::HostThermal {
            chassis_temps_c: vec![65.0, 62.0],
            throttle_thermal: Some(false),
            sm_clock_max_mhz: Some(3003.0),
            mem_total_kb: Some(127_601_452),
        }),
        alerts: vec![NodeAlert {
            kind: AlertKind::ThermalThrottle,
            severity: Severity::Warning,
            detail: "89 °C chassis".into(),
        }],
        hardware_class: Some("gb10".into()),
        atlas_repo: Some(RepoInfo {
            path: "/workspace/atlas".into(),
            remote_name: "avarok".into(),
            remote_url: "git@github.com:Avarok-Cybersecurity/atlas.git".into(),
            head_sha: Some(Sha::parse(SHA).unwrap()),
            fetched_at_s: None,
        }),
        atlas_home: Some("/workspace/.atlas".into()),
        signer_fp: Some("ab12cd34".into()),
        signer_pubkey_hex: None,
        recipes_synced: true,
        built_shas: vec![BuiltSha {
            sha: Sha::parse(SHA).unwrap(),
            binary_sha256: "ff".into(),
            built_at_s: 0,
            bytes: 1,
        }],
        busy: true,
        busy_reason: Some("job jb-1 running".into()),
        running_job: Some(summary(JobState::Running, None)),
        queued: 1,
        queue_depth: 4,
        disk_free_bytes: Metric::reading(50.0 * 1024.0 * 1024.0 * 1024.0),
        min_free_disk_bytes: 0,
        min_free_fraction: 0.85,
        host_free_fraction: Metric::reading(0.42),
        max_run_s: 7200,
    }
}

#[test]
fn the_node_block_says_what_a_scheduler_would_ask_first() {
    let s = node_block("dgx2", &info());
    for needle in [
        "dgx2  spark-43fa (",
        "atlasctl 0.5.0  bench on",
        "gpu      1×NVIDIA GB10  driver 580.95  cuda ?  clock 1500 MHz  temp n/a  mem 128.0 GiB",
        "thermal  chassis 65 °C  throttle none  clock max 3003 MHz  mem 121.7 GiB",
        "class    gb10",
        "repo     /workspace/atlas  avarok=git@github.com:Avarok-Cybersecurity/atlas.git  head 1a0dc88a8c\n",
        "signer   ab12cd34",
        "built    1a0dc88a8c\n",
        "queue    BUSY (job jb-1 running)  1 queued of 4  max run 7200 s",
        "running  jb-1757770000-deadbeef decode-floor @ 1a0dc88a8c (Running)",
        "host     mem free 42 %  disk free 50.0 GiB",
        "ALERT    ThermalThrottle: 89 °C chassis",
    ] {
        assert!(s.contains(needle), "missing {needle:?} in:\n{s}");
    }
    let mut off = info();
    off.bench_enabled = false;
    off.disabled_reason = Some("no bench.yaml".into());
    off.gpu = None;
    assert!(node_block("x", &off).contains("bench OFF — no bench.yaml"));
    assert!(node_block("x", &off).contains("gpu      none reported"));
}

#[test]
fn the_status_table_has_a_header_and_one_line_per_job() {
    assert_eq!(status_table(&[]), "no jobs\n");
    let s = status_table(&[
        summary(JobState::Running, None),
        summary(
            JobState::Done,
            Some(Outcome::Failed {
                stage: JobState::Building,
                reason: "cargo".into(),
            }),
        ),
    ]);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("JOB"));
    assert!(
        lines[1].contains("running")
            && lines[1].contains("decode-floor")
            && lines[1].contains("42")
    );
    assert!(lines[2].contains("done") && lines[2].ends_with("failed while Building: cargo"));
}

fn ev(kind: EventKind) -> BenchEvent {
    BenchEvent {
        job: JobId::parse("jb-1-deadbeef").unwrap(),
        seq: 7,
        at_ms: 3_723_000,
        kind,
    }
}

#[test]
fn each_event_kind_renders_and_heartbeats_render_nothing() {
    let one = |k: EventKind| {
        let v = event_lines(&ev(k));
        assert_eq!(v.len(), 1);
        v.into_iter().next().unwrap()
    };
    assert_eq!(
        one(EventKind::Queued { position: 2 }),
        "[01:02:03 #7] queued at position 2"
    );
    assert!(
        one(EventKind::Preparing {
            sha: Sha::parse(SHA).unwrap(),
            fetched: true
        })
        .ends_with("preparing 1a0dc88a8c (fetched)")
    );
    assert!(
        one(EventKind::Build {
            cached: true,
            reason: "provenance matches".into()
        })
        .ends_with("build: cached (provenance matches)")
    );
    assert!(
        one(EventKind::Built {
            binary_sha256: "0123456789abcdefff".into(),
            cached: false,
            secs: 900
        })
        .ends_with("built 0123456789ab in 900s")
    );
    assert!(
        one(EventKind::Running {
            pid: 12,
            argv: vec!["spark".into(), "benchmark".into()]
        })
        .ends_with("running pid 12: spark benchmark")
    );
    assert!(
        one(EventKind::Progress {
            phase: "isl 512".into(),
            detail: "[3/8]".into()
        })
        .ends_with("isl 512  [3/8]")
    );
    assert_eq!(
        event_lines(&ev(EventKind::Log {
            stream: LogStream::Run,
            lines: vec!["a".into(), "b".into()]
        })),
        vec!["  run| a", "  run| b"]
    );
    assert!(one(EventKind::LogTruncated { dropped_bytes: 9 }).contains("9 bytes of log"));
    assert!(
        one(EventKind::Verdict {
            verdict: Verdict {
                kind: VerdictKind::Pass,
                text: "median 26".into()
            }
        })
        .ends_with("Pass: median 26")
    );
    assert!(
        one(EventKind::Artifact {
            meta: ArtifactMeta {
                name: "r.json".into(),
                relative_path: ".benchmarks/g/r.json".into(),
                bytes: 5,
                sha256: String::new(),
                kind: ArtifactKind::Record
            }
        })
        .ends_with("artifact .benchmarks/g/r.json (5 bytes)")
    );
    assert!(
        one(EventKind::Done {
            outcome: Outcome::Orphaned {
                reason: "restart".into()
            }
        })
        .ends_with("done — orphaned: restart")
    );
    assert!(
        event_lines(&ev(EventKind::Heartbeat {
            state: JobState::Running,
            seq_high: 7
        }))
        .is_empty()
    );
}
