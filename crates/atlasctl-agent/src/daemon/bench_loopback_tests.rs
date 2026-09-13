// SPDX-License-Identifier: AGPL-3.0-only

//! Two real agents over localhost TLS, driving the bench frames through the
//! production serving path with the production client (`peer::bench`).
//!
//! No worker runs, so nothing here depends on the GPU, on `cargo`, or on
//! whether a `spark` is busy on the box: the job is submitted, seen,
//! cancelled while queued, and its journal replayed — the whole of the wire
//! contract that `atlasctl bench` and `spark bench certify` rely on.

use super::relay_harness::*;
use crate::bench::{BenchConfig, BenchHost};
use crate::launcher::Launcher;
use crate::peer::bench::{AttachRefused, DialError, attach, send_bench};
use crate::peer::link::SelfIntro;
use atlasctl_protocol::msg::bench::{GateId, JobKey, JobState, Sha};
use atlasctl_protocol::msg::bench_event::{EventKind, Outcome};
use atlasctl_protocol::msg::{BenchRefusal, BenchRep, BenchReq};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const SHA: &str = "1a0dc88a8c9083bb956bd84cafa2cccbdb8e6e18";

fn bench_config(root: &std::path::Path) -> BenchConfig {
    std::fs::create_dir_all(root.join("repo/.git")).unwrap();
    std::fs::create_dir_all(root.join("home")).unwrap();
    BenchConfig {
        atlas_repo: root.join("repo"),
        atlas_home: root.join("home"),
        hardware: "gb10".into(),
        env: BTreeMap::new(),
        cache_dir: root.join("cache"),
        allowed_remote: "avarok".into(),
        allow_unpublished_shas: false,
        queue_depth: 2,
        max_run_s: 60,
        build_timeout_s: 60,
        stall_timeout_s: 60,
        cancel_grace_s: 1,
        min_free_fraction: 0.0,
        min_free_disk_bytes: 1,
        keep_builds: 1,
        retain_jobs: 5,
        retain_days: 1,
        sync_recipes: false,
        collect_extra: vec![],
    }
}

fn submit(key: &str) -> BenchReq {
    BenchReq::Submit {
        job_key: JobKey::parse(key).unwrap(),
        sha: Sha::parse(SHA).unwrap(),
        gate: GateId::parse("decode-floor").unwrap(),
        params: BTreeMap::new(),
        checkpoint: None,
        hardware: Some("gb10".into()),
        max_run_s: None,
        note: Some("loopback".into()),
    }
}

/// A node with a bench host, and a submitter pinned there with the grant
/// as requested. Returns the node, the submitter and the node's socket.
async fn fleet(tag: &str, granted: bool) -> (TestAgent, TestAgent, std::net::SocketAddr) {
    let mut node = agent(&format!("{tag}-node"), "dgx2", "127.0.0.2");
    let submitter = agent(&format!("{tag}-sub"), "macbook", "127.0.0.1");
    pin(&node, &submitter, false);
    pin(&submitter, &node, false);
    if granted {
        assert!(node.pins.set_bench(submitter.id(), true).unwrap());
    }
    let host = BenchHost::new(
        bench_config(&node.tmp.0),
        node.id(),
        Arc::clone(&node.fleet),
    )
    .expect("host");
    let launcher: Arc<dyn Launcher> = Arc::clone(&node.launcher) as Arc<dyn Launcher>;
    spawn_serving_bench(
        &mut node,
        0,
        Duration::from_secs(5),
        launcher,
        Some(host),
        None,
    )
    .await;
    let sock = node.sock();
    (node, submitter, sock)
}

fn intro() -> SelfIntro {
    SelfIntro::new(false, "")
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_status_cancel_and_attach_replay_over_the_real_channel() {
    let (node, sub, sock) = fleet("bl-happy", true).await;

    // NodeInfo: bench is on, the class is what the config says, nothing runs.
    let (who, rep) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &BenchReq::NodeInfo,
    )
    .await
    .expect("node info");
    assert_eq!(who, node.id());
    let BenchRep::NodeInfo { info } = rep else {
        panic!("{rep:?}")
    };
    assert!(info.bench_enabled, "{info:?}");
    assert_eq!(info.hardware_class.as_deref(), Some("gb10"));
    assert_eq!(info.node, node.id());
    assert!(info.running_job.is_none());

    // Submit, twice with the same key: one job.
    let (_, first) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &submit("k1"),
    )
    .await
    .expect("submit");
    let BenchRep::Accepted {
        job,
        existing: false,
        state: JobState::Queued,
        ..
    } = first
    else {
        panic!("{first:?}")
    };
    let (_, again) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &submit("k1"),
    )
    .await
    .expect("resubmit");
    match again {
        BenchRep::Accepted {
            job: j2,
            existing: true,
            ..
        } => assert_eq!(j2, job),
        other => panic!("{other:?}"),
    }
    // NEGATIVE CONTROL: the same key with a different gate is a conflict,
    // not a silent second job.
    let mut other_gate = submit("k1");
    if let BenchReq::Submit { gate, .. } = &mut other_gate {
        *gate = GateId::parse("ttft-warm-gate").unwrap();
    }
    let (_, conflict) = send_bench(&sub.identity, sub.pins.clone(), sock, &intro(), &other_gate)
        .await
        .expect("conflict reply");
    assert!(
        matches!(
            conflict,
            BenchRep::Refused {
                refusal: BenchRefusal::KeyConflict { .. },
                ..
            }
        ),
        "{conflict:?}"
    );

    // Status lists it, queued.
    let (_, st) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &BenchReq::Status {
            job: Some(job.clone()),
        },
    )
    .await
    .expect("status");
    let BenchRep::Status { jobs } = st else {
        panic!("{st:?}")
    };
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, JobState::Queued);
    assert_eq!(jobs[0].gate.as_str(), "decode-floor");

    // Cancel while queued: terminal at once, and idempotent.
    for _ in 0..2 {
        let (_, c) = send_bench(
            &sub.identity,
            sub.pins.clone(),
            sock,
            &intro(),
            &BenchReq::Cancel { job: job.clone() },
        )
        .await
        .expect("cancel");
        assert!(
            matches!(
                c,
                BenchRep::Cancelled {
                    state: JobState::Done,
                    ..
                }
            ),
            "{c:?}"
        );
    }

    // Attach from the start replays the journal: Queued, then Done/Cancelled.
    let mut stream = attach(&sub.identity, sub.pins.clone(), sock, &intro(), &job, 1)
        .await
        .expect("attach");
    let mut kinds = vec![];
    while let Some(ev) = stream.next().await.expect("event") {
        assert_eq!(ev.job, job);
        kinds.push(ev.kind);
    }
    assert!(
        matches!(kinds.first(), Some(EventKind::Queued { .. })),
        "{kinds:?}"
    );
    assert!(
        matches!(
            kinds.last(),
            Some(EventKind::Done {
                outcome: Outcome::Cancelled { by }
            }) if *by == node.id()
        ),
        "{kinds:?}"
    );
    let last_seq = stream.last_seq;
    assert_eq!(last_seq as usize, kinds.len());

    // Re-attaching past the end yields nothing new — the resume contract.
    let mut resumed = attach(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &job,
        last_seq + 1,
    )
    .await
    .expect("re-attach");
    // A finished job's stream ends with a Done replay or nothing; either
    // way no event below `from_seq` arrives.
    while let Some(ev) = resumed.next().await.expect("event") {
        assert!(ev.seq > last_seq, "replayed {ev:?} below from_seq");
    }

    // Artifacts of a cancelled-while-queued job: none (no child ever ran).
    let (_, arts) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &BenchReq::ArtifactList { job: job.clone() },
    )
    .await
    .expect("artifacts");
    let BenchRep::Artifacts { items, .. } = arts else {
        panic!("{arts:?}")
    };
    assert!(items.is_empty(), "{items:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_but_ungranted_submitter_is_refused_with_the_grant_command() {
    let (node, sub, sock) = fleet("bl-nogrant", false).await;
    let (_, rep) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &submit("k"),
    )
    .await
    .expect("a reply, not a dropped link");
    match rep {
        BenchRep::Refused {
            by,
            refusal: BenchRefusal::NotGranted { command },
        } => {
            assert_eq!(by, node.id());
            assert!(command.contains("grant-bench"), "{command}");
            assert!(command.contains(&sub.id().short()), "{command}");
        }
        other => panic!("{other:?}"),
    }
    // NEGATIVE CONTROL: nothing was accepted — the job list is empty.
    // (Status itself needs the grant too, so ask through NodeInfo... which
    // is also gated; the store on disk is the witness.)
    assert!(
        std::fs::read_dir(node.tmp.0.join("cache/jobs"))
            .map(|d| d
                .filter_map(Result::ok)
                .filter(|e| e.file_name() != "by-key")
                .count()
                == 0)
            .unwrap_or(true)
    );
    // An attach by an ungranted peer is refused too, as a typed error.
    let mut s = attach(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &atlasctl_protocol::msg::bench::JobId::parse("jb-1-deadbeef").unwrap(),
        1,
    )
    .await
    .expect("attach sends");
    let e = s.next().await.expect_err("refused");
    assert!(e.downcast_ref::<AttachRefused>().is_some(), "{e:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unpinned_caller_never_gets_past_the_handshake() {
    let (_node, _sub, sock) = fleet("bl-stranger", true).await;
    let stranger = agent("bl-stranger-x", "laptop", "127.0.0.9");
    // The stranger has no pin for the node: its own verifier refuses.
    let e = send_bench(
        &stranger.identity,
        stranger.pins.clone(),
        sock,
        &intro(),
        &BenchReq::NodeInfo,
    )
    .await
    .expect_err("refused");
    let d = e.downcast_ref::<DialError>().expect("typed");
    assert!(d.not_paired(), "{d}");
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_grant_cuts_an_attached_stream_at_its_next_event() {
    let (node, sub, sock) = fleet("bl-revoke", true).await;
    let (_, rep) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &submit("k-revoke"),
    )
    .await
    .expect("submit");
    let BenchRep::Accepted { job, .. } = rep else {
        panic!("{rep:?}")
    };
    // Attach while the job is queued (no worker: it stays queued), read the
    // Queued event, then revoke.
    let mut stream = attach(&sub.identity, sub.pins.clone(), sock, &intro(), &job, 1)
        .await
        .expect("attach");
    let first = stream.next().await.expect("event").expect("queued");
    assert!(matches!(first.kind, EventKind::Queued { .. }));
    assert!(node.pins.set_bench(sub.id(), false).expect("revoke"));
    // Something has to wake the server's loop: cancelling the job appends
    // a Done event. The stream must end on the refusal, never deliver it.
    let (_, c) = send_bench(
        &sub.identity,
        sub.pins.clone(),
        sock,
        &intro(),
        &BenchReq::Cancel { job: job.clone() },
    )
    .await
    .expect("cancel reply");
    // NEGATIVE CONTROL for the revocation itself: the cancel is refused
    // too, since the grant is gone for every frame.
    assert!(
        matches!(
            c,
            BenchRep::Refused {
                refusal: BenchRefusal::NotGranted { .. },
                ..
            }
        ),
        "{c:?}"
    );
    // The server loop wakes on the journal or its heartbeat tick; either
    // way the next thing the client sees is the refusal.
    let e = tokio::time::timeout(Duration::from_secs(20), stream.next())
        .await
        .expect("within a heartbeat")
        .expect_err("cut");
    assert!(e.downcast_ref::<AttachRefused>().is_some(), "{e:#}");
}
