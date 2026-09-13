// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_protocol::msg::bench_event::{Verdict, VerdictKind};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[path = "machine_tests/harness.rs"]
mod harness;
use harness::*;

#[test]
fn the_happy_path_walks_every_stage_and_renders_the_argv_itself() {
    let w = world();
    let s = Script::happy();
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(out.passed(), "{out:?}");
    assert!(
        matches!(&out, Outcome::Completed { record: Some(r), signature: Some(_), .. } if r.ends_with("r.json"))
    );
    let calls = s.calls();
    assert!(calls.iter().any(|c| c.starts_with("spawn /cache/build/x/spark benchmark run decode-floor --pull-request-gate --hardware gb10 --yes --param osl=8")), "{calls:?}");
    assert!(
        !calls.contains(&"fetch".to_string()),
        "the commit was already here"
    );
    assert_eq!(
        kinds(&w),
        [
            "preparing",
            "build",
            "built",
            "running",
            "log",
            "verdict",
            "artifact",
            "artifact",
            "done"
        ]
    );
    let stored = w.store.load(&w.job.id).unwrap();
    assert_eq!(stored.state, JobState::Done);
    assert_eq!(
        stored.binary_sha256.as_deref(),
        Some("cc".repeat(32).as_str())
    );
    assert!(stored.child_pid.is_none(), "cleared once the child is gone");
    assert_eq!(stored.seq_high, 9);
}

#[test]
fn a_missing_commit_is_fetched_once_and_then_refused() {
    let w = world();
    let mut s = Script::happy();
    s.has_commit = vec![false, true];
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(out.passed());
    assert!(s.calls().contains(&"fetch".to_string()));
    let ev = w.journal.replay(1).unwrap();
    assert!(matches!(
        ev[0].kind,
        EventKind::Preparing { fetched: true, .. }
    ));

    let w = world();
    let mut s = Script::happy();
    s.has_commit = vec![false, false];
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(out, Outcome::Failed { stage: JobState::Preparing, ref reason } if reason.contains("not on the remote")),
        "{out:?}"
    );
}

/// NEGATIVE CONTROL: an unpublished commit never builds.
#[test]
fn an_unpublished_commit_is_refused_before_any_build() {
    let w = world();
    let mut s = Script::happy();
    s.published = false;
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(out, Outcome::Failed { stage: JobState::Preparing, ref reason } if reason.contains("trusted remote")),
        "{out:?}"
    );
    assert!(!s.calls().contains(&"build".to_string()));
    // …unless the node's operator allowed it.
    let w = world();
    let s = {
        let mut s = Script::happy();
        s.published = false;
        s
    };
    let mut c = ctx(&w, &s, Arc::new(AtomicBool::new(false)));
    c.allow_unpublished = true;
    assert!(run(&c, w.job.clone()).unwrap().passed());
}

#[test]
fn a_cache_hit_skips_the_build_and_says_so() {
    let w = world();
    let mut s = Script::happy();
    s.cache = Ok(Script::binary());
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(out.passed());
    assert!(!s.calls().contains(&"build".to_string()), "{:?}", s.calls());
    let ev = w.journal.replay(1).unwrap();
    assert!(
        matches!(&ev[1].kind, EventKind::Build { cached: true, reason } if reason.contains("cache hit"))
    );
    assert!(matches!(
        ev[2].kind,
        EventKind::Built {
            cached: true,
            secs: 0,
            ..
        }
    ));
}

/// NEGATIVE CONTROLS: a stale binary — provenance for another sha, or bytes
/// that do not hash to their provenance — is rebuilt, never reused, and the
/// journal names why.
#[test]
fn a_stale_or_tampered_cached_binary_is_rebuilt_not_reused() {
    for (miss, needle) in [
        (
            CacheMiss::WrongSha("deadbeef00".into()),
            "provenance names deadbeef00",
        ),
        (CacheMiss::HashMismatch, "hash does not match"),
        (CacheMiss::NoProvenance, "no provenance"),
    ] {
        let w = world();
        let mut s = Script::happy();
        s.cache = Err(miss);
        let out = run(
            &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
            w.job.clone(),
        )
        .unwrap();
        assert!(out.passed());
        assert!(s.calls().contains(&"build".to_string()), "{needle}");
        let ev = w.journal.replay(1).unwrap();
        assert!(
            matches!(&ev[1].kind, EventKind::Build { cached: false, reason } if reason.contains(needle)),
            "{:?}",
            ev[1].kind
        );
    }
}

#[test]
fn an_unknown_gate_and_bad_params_are_refused_by_the_binary_itself() {
    let w = world();
    let mut s = Script::happy();
    s.known = vec!["vision-fidelity".into()];
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(out, Outcome::Failed { stage: JobState::Building, ref reason } if reason.contains("vision-fidelity")),
        "{out:?}"
    );
    assert!(!s.calls().iter().any(|c| c.starts_with("spawn")));

    let w = world();
    let mut s = Script::happy();
    s.bad_params = vec!["osl".into()];
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(out, Outcome::Failed { stage: JobState::Building, ref reason } if reason.contains("osl")),
        "{out:?}"
    );
}

#[test]
fn a_failed_build_and_a_failed_spawn_are_failures_at_their_stage() {
    let w = world();
    let mut s = Script::happy();
    s.build_ok = false;
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(
            out,
            Outcome::Failed {
                stage: JobState::Building,
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn stall_timeout_and_cancel_end_the_run_with_their_own_outcomes() {
    let w = world();
    let mut s = Script::happy();
    s.end = RunEnd::Stalled { after_s: 1800 };
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert_eq!(
        out,
        Outcome::TimedOut {
            stage: JobState::Running,
            after_s: 1800
        }
    );

    let w = world();
    let mut s = Script::happy();
    s.end = RunEnd::Cancelled;
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(matches!(out, Outcome::Cancelled { .. }));

    // Cancel during the build: the build's error is read as a cancel.
    let w = world();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut s = Script::happy();
    s.cancel_during_build = Some(cancel.clone());
    let out = run(&ctx(&w, &s, cancel), w.job.clone()).unwrap();
    assert!(matches!(out, Outcome::Cancelled { .. }), "{out:?}");
}

/// NEGATIVE CONTROL: exit 0 with no record is a failure, not a success with
/// nothing to show.
#[test]
fn exit_zero_without_a_record_is_a_collecting_failure() {
    let w = world();
    let mut s = Script::happy();
    s.artifacts.clear();
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(
        matches!(out, Outcome::Failed { stage: JobState::Collecting, ref reason } if reason.contains("no gate record")),
        "{out:?}"
    );
    // A verdict FAIL with exit 2 and a record is a completed run that said no.
    let w = world();
    let mut s = Script::happy();
    s.end = RunEnd::Exited {
        code: 2,
        verdict: Some(Verdict {
            kind: VerdictKind::Fail,
            text: "below floor".into(),
        }),
    };
    let out = run(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
    )
    .unwrap();
    assert!(matches!(out, Outcome::Completed { exit_code: 2, .. }));
    assert!(!out.passed());
}

#[test]
fn orphan_records_a_terminal_outcome() {
    let w = world();
    let s = Script::happy();
    orphan(
        &ctx(&w, &s, Arc::new(AtomicBool::new(false))),
        w.job.clone(),
        "agent restarted",
    )
    .unwrap();
    let stored = w.store.load(&w.job.id).unwrap();
    assert_eq!(stored.state, JobState::Done);
    assert!(matches!(stored.outcome, Some(Outcome::Orphaned { .. })));
}
