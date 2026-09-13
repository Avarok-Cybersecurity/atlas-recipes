// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_agent::peer::tls::NOT_PAIRED_MARKER;
use atlasctl_protocol::msg::bench::{JobId, JobState};
use atlasctl_protocol::msg::bench_event::{Verdict, VerdictKind};
use std::net::SocketAddr;
use std::time::Duration;

fn node() -> NodeId {
    NodeId::from_bytes([1; 32])
}

fn addr() -> SocketAddr {
    "10.10.10.2:34334".parse().unwrap()
}

#[test]
fn every_refusal_has_a_code_and_the_grant_one_is_a_pairing_failure() {
    let cases: Vec<(BenchRefusal, &str, Code, bool)> = vec![
        (
            BenchRefusal::NotGranted {
                command: "atlasctl peer grant-bench ab12".into(),
            },
            "not_granted",
            Code::NotPaired,
            false,
        ),
        (
            BenchRefusal::Busy {
                what: "spark is running".into(),
                retry_after_s: 60,
            },
            "refused:busy",
            Code::Refused,
            true,
        ),
        (
            BenchRefusal::QueueFull {
                depth: 2,
                retry_after_s: 30,
            },
            "refused:queue_full",
            Code::Refused,
            true,
        ),
        (
            BenchRefusal::RateLimited { retry_after_s: 5 },
            "refused:rate_limited",
            Code::Refused,
            true,
        ),
        (
            BenchRefusal::MemoryPressure {
                available_frac: 0.5,
                required_frac: 0.85,
            },
            "refused:memory_pressure",
            Code::Refused,
            true,
        ),
        (
            BenchRefusal::UnknownGate {
                gate: "nope".into(),
                known: vec!["decode-floor".into(), "ttft-warm-gate".into()],
            },
            "refused:unknown_gate",
            Code::Refused,
            false,
        ),
        (
            BenchRefusal::KeyConflict {
                job: JobId::parse("jb-1-deadbeef").unwrap(),
            },
            "refused:key_conflict",
            Code::Refused,
            false,
        ),
        (
            BenchRefusal::ShaNotAllowed {
                reason: "not on avarok".into(),
            },
            "refused:sha_not_allowed",
            Code::Refused,
            false,
        ),
        (
            BenchRefusal::UnknownJob {
                job: JobId::parse("jb-1-deadbeef").unwrap(),
            },
            "refused:unknown_job",
            Code::Refused,
            false,
        ),
        (
            BenchRefusal::Unsupported { version_max: 2 },
            "refused:unsupported",
            Code::Refused,
            false,
        ),
    ];
    for (refusal, code, exit, retryable) in cases {
        let e = BenchError::refused(node(), &refusal);
        assert_eq!(e.obj.code, code);
        assert_eq!(e.exit, exit, "{code}");
        assert_eq!(e.obj.retryable, retryable, "{code}");
        assert_eq!(e.obj.node_id.as_deref(), Some(node().to_string().as_str()));
        assert!(!e.obj.message.is_empty());
    }
    // The fix carries what the node said to run, verbatim.
    let e = BenchError::refused(
        node(),
        &BenchRefusal::NotGranted {
            command: "atlasctl peer grant-bench ab12".into(),
        },
    );
    assert_eq!(
        e.obj.fix.as_deref(),
        Some("on the node, run: atlasctl peer grant-bench ab12")
    );
    let e = BenchError::refused(
        node(),
        &BenchRefusal::UnknownGate {
            gate: "nope".into(),
            known: vec!["a".into(), "b".into()],
        },
    );
    assert_eq!(e.obj.fix.as_deref(), Some("one of: a, b"));
}

#[test]
fn outcomes_map_to_failed_or_cancelled_exit_codes() {
    let done_pass = Outcome::Completed {
        exit_code: 0,
        verdict: Some(Verdict {
            kind: VerdictKind::Pass,
            text: "ok".into(),
        }),
        record: Some("r.json".into()),
        signature: None,
    };
    assert!(done_pass.passed());
    let done_fail = Outcome::Completed {
        exit_code: 2,
        verdict: Some(Verdict {
            kind: VerdictKind::Fail,
            text: "below floor".into(),
        }),
        record: Some("r.json".into()),
        signature: None,
    };
    let e = BenchError::outcome(node(), &done_fail);
    assert_eq!(
        (e.exit, e.obj.code.as_str()),
        (Code::JobFailed, "job_failed")
    );
    assert!(
        e.obj.message.contains("Fail: below floor"),
        "{}",
        e.obj.message
    );
    // NEGATIVE CONTROL: exit 0 with no verdict is not a pass and reads so.
    let no_verdict = Outcome::Completed {
        exit_code: 0,
        verdict: None,
        record: None,
        signature: None,
    };
    assert!(!no_verdict.passed());
    assert!(
        BenchError::outcome(node(), &no_verdict)
            .obj
            .message
            .contains("no verdict")
    );
    let e = BenchError::outcome(node(), &Outcome::Cancelled { by: node() });
    assert_eq!(
        (e.exit, e.obj.code.as_str()),
        (Code::Cancelled, "job_cancelled")
    );
    for o in [
        Outcome::Failed {
            stage: JobState::Building,
            reason: "cargo".into(),
        },
        Outcome::TimedOut {
            stage: JobState::Running,
            after_s: 10,
        },
        Outcome::Orphaned {
            reason: "restart".into(),
        },
    ] {
        assert_eq!(BenchError::outcome(node(), &o).exit, Code::JobFailed);
    }
}

#[test]
fn link_errors_are_classified_by_type_not_by_prose() {
    let timeout = anyhow::Error::from(DialError::Timeout {
        addr: addr(),
        timeout: Duration::from_secs(5),
    });
    let e = BenchError::from_link("dgx2", timeout);
    assert_eq!(
        (e.exit, e.obj.code.as_str()),
        (Code::Unreachable, "unreachable")
    );
    assert!(e.obj.retryable);
    assert_eq!(e.obj.node.as_deref(), Some("dgx2"));

    let refused = anyhow::Error::from(DialError::Connect {
        addr: addr(),
        source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
    });
    assert_eq!(
        BenchError::from_link("dgx2", refused).exit,
        Code::Unreachable
    );

    // Our verifier refused the peer: the message carries the marker.
    let ours = anyhow::Error::from(DialError::Handshake {
        addr: addr(),
        source: std::io::Error::other(format!("peer abcd {NOT_PAIRED_MARKER}")),
    });
    let e = BenchError::from_link("dgx2", ours);
    assert_eq!(
        (e.exit, e.obj.code.as_str()),
        (Code::NotPaired, "not_paired")
    );
    assert!(!e.obj.retryable);
    assert!(e.obj.fix.as_deref().unwrap().contains("grant-bench"));

    // The other side hung up during the handshake: also a pairing problem.
    let theirs = anyhow::Error::from(DialError::Handshake {
        addr: addr(),
        source: std::io::Error::other("received fatal alert: BadCertificate"),
    });
    let e = BenchError::from_link("dgx2", theirs);
    assert_eq!(e.exit, Code::NotPaired);
    assert!(e.obj.message.contains("may not have this machine pinned"));

    // Wrapped in context, the type still comes through.
    let wrapped = anyhow::Error::from(UnsupportedPeer {
        name: "spark-43fa".into(),
        peer: node(),
        version_max: 2,
    })
    .context("dialling");
    let e = BenchError::from_link("dgx2", wrapped);
    assert_eq!(
        (e.exit, e.obj.code.as_str()),
        (Code::Unsupported, "unsupported_version")
    );

    // NEGATIVE CONTROL: prose alone is "unreachable", never "not paired".
    let prose = anyhow::anyhow!("peer xyz {NOT_PAIRED_MARKER}");
    assert_eq!(BenchError::from_link("dgx2", prose).exit, Code::Unreachable);
}

#[test]
fn the_json_shape_omits_absent_fields_and_the_text_carries_the_fix() {
    let e = BenchError::unreachable("10.10.10.2:34334 did not answer within 5s").at("dgx2");
    let json = serde_json::to_value(&e.obj).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "code": "unreachable",
            "message": "10.10.10.2:34334 did not answer within 5s",
            "node": "dgx2",
            "retryable": true
        })
    );
    let e = BenchError::not_paired("peer ab is not paired")
        .at("dgx2")
        .by(node());
    let text = e.to_string();
    assert!(
        text.starts_with("dgx2: peer ab is not paired\n  fix: "),
        "{text}"
    );
    let json = serde_json::to_value(&e.obj).unwrap();
    assert!(json.get("fix").is_some() && json.get("node_id").is_some());
    assert_eq!(Code::Usage.exit(), std::process::ExitCode::from(1));
    assert_eq!(Code::Unsupported.exit(), std::process::ExitCode::from(8));
}

#[test]
fn an_unexpected_refused_reply_is_that_refusal() {
    let rep = BenchRep::Refused {
        by: node(),
        refusal: BenchRefusal::UnknownJob {
            job: JobId::parse("jb-1-deadbeef").unwrap(),
        },
    };
    let e = BenchError::unexpected("dgx2", &rep);
    assert_eq!(e.obj.code, "refused:unknown_job");
    let e = BenchError::unexpected("dgx2", &BenchRep::Status { jobs: vec![] });
    assert_eq!(e.obj.code, "unreachable");
    assert!(e.obj.message.contains("unexpected reply"));
}
