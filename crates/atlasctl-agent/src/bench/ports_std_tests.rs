// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::bench::child::{parse_progress, parse_verdict, sanitize};
use atlasctl_protocol::msg::bench::MAX_LOG_LINE_BYTES;
use atlasctl_protocol::msg::bench_event::VerdictKind;
use std::sync::Arc;

#[test]
fn the_verdict_line_is_parsed_exactly_as_atlas_prints_it() {
    let v = parse_verdict("  Pass: median decode 26.0 tok/s").unwrap();
    assert_eq!(v.kind, VerdictKind::Pass);
    assert_eq!(v.text, "median decode 26.0 tok/s");
    assert_eq!(
        parse_verdict("  Fail: BELOW THE BASELINE").unwrap().kind,
        VerdictKind::Fail
    );
    assert_eq!(
        parse_verdict("  Info: not scored").unwrap().kind,
        VerdictKind::Info
    );
    // NEGATIVE CONTROLS: prose that mentions the word, and log noise.
    assert!(parse_verdict("this Pass: is not a verdict").is_none());
    assert!(parse_verdict("2026-09-13 INFO spark: ready").is_none());
    assert!(parse_verdict("  PASS  decode-floor").is_none());
}

#[test]
fn progress_lines_become_progress_events() {
    match parse_progress("  [  12.3s] isl 512 · conc 8 [3/8]").unwrap() {
        EventKind::Progress { phase, .. } => assert_eq!(phase, "isl 512 · conc 8 [3/8]"),
        other => panic!("{other:?}"),
    }
    assert!(parse_progress("plain line").is_none());
    assert!(
        parse_progress("  [  1.0s]").is_none(),
        "an empty phase is not progress"
    );
}

#[test]
fn log_lines_are_sanitised_and_bounded() {
    assert_eq!(sanitize("a\x1b[31mb\x07c"), "a[31mbc");
    let long = "x".repeat(MAX_LOG_LINE_BYTES + 50);
    let s = sanitize(&long);
    assert!(s.chars().count() <= MAX_LOG_LINE_BYTES + 1);
    assert!(s.ends_with('…'));
}

#[test]
fn start_ticks_reads_our_own_process_and_identifies_it() {
    let me = std::process::id();
    let ticks = start_ticks(me).expect("/proc/self/stat is readable here");
    assert!(same_process(me, ticks));
    assert!(
        !same_process(me, ticks + 1),
        "a different start time is a different process"
    );
}

/// The cache rule, end to end on a scratch cache: provenance for another sha
/// and a tampered binary are both misses.
#[test]
fn a_cached_binary_is_a_hit_only_when_provenance_and_bytes_agree() {
    let dir = std::env::temp_dir().join(format!("bench-ports-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo/.git")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let cfg = BenchConfig {
        atlas_repo: dir.join("repo"),
        atlas_home: dir.join("home"),
        hardware: "gb10".into(),
        env: Default::default(),
        cache_dir: dir.join("cache"),
        allowed_remote: "origin".into(),
        allow_unpublished_shas: false,
        queue_depth: 1,
        max_run_s: 10,
        build_timeout_s: 10,
        stall_timeout_s: 10,
        cancel_grace_s: 1,
        min_free_fraction: 0.5,
        min_free_disk_bytes: 1,
        keep_builds: 1,
        retain_jobs: 1,
        retain_days: 1,
        sync_recipes: false,
        collect_extra: vec![],
    };
    let ports = StdPorts::new(cfg);
    let sha = Sha::parse(&"ab".repeat(20)).unwrap();
    assert_eq!(ports.cached_binary(&sha).unwrap(), Err(CacheMiss::NoBuild));

    let bdir = ports.build_dir(&sha);
    std::fs::create_dir_all(&bdir).unwrap();
    std::fs::write(bdir.join("spark"), b"binary bytes").unwrap();
    assert_eq!(
        ports.cached_binary(&sha).unwrap(),
        Err(CacheMiss::NoProvenance)
    );

    let (good_hash, bytes) = sha256_file(&bdir.join("spark")).unwrap();
    let write_prov = |sha: &Sha, hash: &str| {
        let p = Provenance {
            sha: sha.clone(),
            binary_sha256: hash.into(),
            built_at_s: 1,
            bytes,
        };
        write_atomic(
            &bdir.join("provenance.json"),
            &serde_json::to_vec(&p).unwrap(),
        )
        .unwrap();
    };
    let other = Sha::parse(&"cd".repeat(20)).unwrap();
    write_prov(&other, &good_hash);
    assert!(matches!(
        ports.cached_binary(&sha).unwrap(),
        Err(CacheMiss::WrongSha(_))
    ));

    write_prov(&sha, &good_hash);
    assert!(
        ports.cached_binary(&sha).unwrap().is_ok(),
        "provenance and bytes agree"
    );

    std::fs::write(bdir.join("spark"), b"tampered").unwrap();
    assert_eq!(
        ports.cached_binary(&sha).unwrap(),
        Err(CacheMiss::HashMismatch)
    );
}

/// A real child in its own process group: its output is tailed into events,
/// its verdict line is read, and a cancel kills it.
#[test]
fn spawn_and_wait_read_a_real_child() {
    let dir = std::env::temp_dir().join(format!("bench-child-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("repo/.git")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    let cfg = BenchConfig {
        atlas_repo: dir.join("repo"),
        atlas_home: dir.join("home"),
        hardware: "gb10".into(),
        env: Default::default(),
        cache_dir: dir.join("cache"),
        allowed_remote: "origin".into(),
        allow_unpublished_shas: false,
        queue_depth: 1,
        max_run_s: 30,
        build_timeout_s: 10,
        stall_timeout_s: 30,
        cancel_grace_s: 1,
        min_free_fraction: 0.5,
        min_free_disk_bytes: 1,
        keep_builds: 1,
        retain_jobs: 1,
        retain_days: 1,
        sync_recipes: false,
        collect_extra: vec![],
    };
    let ports = StdPorts::new(cfg);
    let plan = RunPlan {
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "echo '  [  1.0s] warmup'; echo '  Fail: below'; exit 2".into(),
        ],
        cwd: dir.clone(),
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        log_path: dir.join("child.log"),
        stall_timeout: Duration::from_secs(30),
        max_run: Duration::from_secs(30),
    };
    let child = ports.spawn(&plan).unwrap();
    let events = std::sync::Mutex::new(Vec::new());
    let end = ports
        .wait(&child, &plan, &AtomicBool::new(false), &|k| {
            events.lock().unwrap().push(k)
        })
        .unwrap();
    match end {
        RunEnd::Exited { code, verdict } => {
            assert_eq!(code, 2);
            assert_eq!(verdict.unwrap().kind, VerdictKind::Fail);
        }
        other => panic!("{other:?}"),
    }
    let ev = events.lock().unwrap();
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Progress { phase, .. } if phase == "warmup")),
        "{ev:?}"
    );
    assert!(ev.iter().any(|e| matches!(e, EventKind::Log { .. })));

    // Cancel: a sleeping child is terminated within the grace period.
    let plan = RunPlan {
        argv: vec!["/bin/sh".into(), "-c".into(), "sleep 20".into()],
        log_path: dir.join("child2.log"),
        ..plan
    };
    let child = ports.spawn(&plan).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        c2.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let end = ports.wait(&child, &plan, &cancel, &|_| {}).unwrap();
    assert_eq!(end, RunEnd::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
}
