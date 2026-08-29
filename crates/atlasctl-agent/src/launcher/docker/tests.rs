// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::host::PosixUser;
use atlasctl_core::io::RecordingRunner;
use atlasctl_core::io::process::Output;
use atlasctl_core::recipe::Provenance;

/// A cache root that really holds `org/m`'s weights.
///
/// The launcher refuses to start a recipe whose model is missing from the cache,
/// because the launch runs offline and the container would otherwise fail after
/// the image pull with nothing useful on screen. These tests are about the
/// docker command rather than the cache, so they need one that satisfies that
/// guard — and building it here, rather than weakening the guard for tests,
/// keeps the production path honest.
fn cache_with_model() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("atlasctl-launch-cache-{}", std::process::id()));
        let snap = root.join("hub/models--org--m/snapshots/deadbeef");
        std::fs::create_dir_all(&snap).expect("mkdir cache");
        std::fs::write(snap.join("config.json"), b"{}").expect("write config");
        std::fs::write(snap.join("model.safetensors"), b"weights").expect("write weights");
        root
    })
}

fn host() -> HostSnapshot {
    HostSnapshot {
        posix_user: Some(PosixUser {
            uid: 1000,
            gid: 1000,
        }),
        home: "/home/spark".into(),
        hf_cache_dir: cache_with_model().display().to_string(),
        env: BTreeMap::new(),
    }
}

fn recipe() -> Recipe {
    Recipe::parse(
        "r",
        "model: org/m\ncontainer: img:tag\nruntime: atlas\ndefaults:\n  port: 8888\n",
        Provenance::Builtin {
            path: "r.yaml".into(),
        },
    )
    .expect("fixture parses")
}

fn launcher(runner: Arc<RecordingRunner>) -> DockerLauncher {
    DockerLauncher::new(runner, host(), &ROOTLESS_V1, Box::new(NvidiaDevices))
}

#[test]
fn preview_renders_the_command_without_running_anything() {
    let runner = Arc::new(RecordingRunner::new());
    let p = launcher(runner.clone())
        .preview(&recipe(), &BTreeMap::new())
        .expect("previews");
    assert!(p.command.starts_with("docker run"), "{}", p.command);
    assert!(p.command.contains("spark serve org/m"));
    assert_eq!(
        runner.call_count(),
        0,
        "preview must not execute anything at all"
    );
}

#[test]
fn preview_and_launch_render_the_same_command() {
    // What the client is shown must be what runs, or inspection is theatre.
    let runner = Arc::new(RecordingRunner::new());
    let l = launcher(runner.clone());
    let previewed = l.preview(&recipe(), &BTreeMap::new()).unwrap().command;
    l.launch(&recipe(), &BTreeMap::new()).unwrap();

    let run_call = runner
        .calls()
        .into_iter()
        .find(|c| {
            c.first().map(String::as_str) == Some("docker")
                && c.get(1).map(String::as_str) == Some("run")
        })
        .expect("a docker run happened");
    // The preview is shell-quoted; compare the meaningful tail.
    assert!(previewed.contains("spark serve org/m"));
    assert!(
        run_call
            .windows(3)
            .any(|w| w == ["spark", "serve", "org/m"])
    );
}

#[test]
fn launching_removes_a_stale_container_first_then_runs() {
    let runner = Arc::new(RecordingRunner::new());
    launcher(runner.clone())
        .launch(&recipe(), &BTreeMap::new())
        .expect("launches");
    let calls = runner.calls();
    assert_eq!(
        calls[0][..3],
        ["docker", "rm", "-f"],
        "stale container is cleared first"
    );
    assert_eq!(calls[0][3], "atlas-r");
    assert_eq!(calls[1][..2], ["docker", "run"]);
}

#[test]
fn a_failed_docker_run_is_reported_with_its_stderr() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 0,
        stdout: String::new(),
        stderr: String::new(),
    }); // rm
    runner.push_result(Output {
        status: 125,
        stdout: String::new(),
        stderr: "no such image".into(),
    });
    let err = launcher(runner)
        .launch(&recipe(), &BTreeMap::new())
        .expect_err("must fail");
    let msg = err.to_string();
    assert!(msg.contains("125"), "{msg}");
    assert!(msg.contains("no such image"), "{msg}");
}

#[test]
fn the_endpoint_reflects_the_resolved_port() {
    let runner = Arc::new(RecordingRunner::new());
    let started = launcher(runner)
        .launch(&recipe(), &BTreeMap::new())
        .expect("launches");
    assert_eq!(started.container, "atlas-r");
    assert_eq!(
        started.endpoint.as_deref(),
        Some("http://localhost:8888/v1")
    );
}

#[test]
fn an_override_reaches_the_executed_command() {
    let runner = Arc::new(RecordingRunner::new());
    let overrides = BTreeMap::from([("port".to_string(), ScalarValue::Int(9100))]);
    let started = launcher(runner.clone())
        .launch(&recipe(), &overrides)
        .expect("launches");
    assert_eq!(
        started.endpoint.as_deref(),
        Some("http://localhost:9100/v1")
    );
    let run = runner
        .calls()
        .into_iter()
        .find(|c| c.get(1).map(String::as_str) == Some("run"))
        .unwrap();
    assert!(run.windows(2).any(|w| w == ["--port", "9100"]));
}

/// Nothing labelled: fall back to our own name, and never a wider pattern.
///
/// A container started by an older agent predates the recipe label, and must
/// still be stoppable -- but the fallback is an exact name, so an unrelated
/// container can never be caught by it.
#[test]
fn stop_falls_back_to_our_own_container_name_when_nothing_is_labelled() {
    let runner = Arc::new(RecordingRunner::new());
    launcher(runner.clone()).stop("r").expect("stops");
    let calls = runner.calls();
    assert!(
        calls[0].iter().any(|a| a.contains("io.atlasctl.recipe=r")),
        "must ask docker by OUR label first: {calls:?}"
    );
    assert_eq!(
        calls.last().expect("a stop"),
        &["docker", "stop", "atlas-r"],
        "and fall back to the exact name, not a pattern: {calls:?}"
    );
}

/// A cluster rank is `atlas-{recipe}-rank{n}`, so stopping by NAME never found
/// one. That is the state after a head agent restarts mid-cluster: the driver's
/// record is memory-only and gone, `StopCluster` says it never started one, and
/// this path was the only other way to reach the containers. Both GPUs held,
/// no button that works.
#[test]
fn stop_reaches_rank_containers_a_name_match_would_miss() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 0,
        stdout: "abc123\ndef456\n".into(),
        stderr: String::new(),
    });
    launcher(runner.clone()).stop("r").expect("stops");
    let calls = runner.calls();
    assert_eq!(
        calls.last().expect("a stop"),
        &["docker", "stop", "abc123", "def456"],
        "every labelled container must be stopped, ranks included: {calls:?}"
    );
}

#[test]
fn running_launches_are_found_by_label_and_mapped_back_to_recipes() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 0,
        stdout: "atlas-qwen3.6-27b-fp8\tUp 3 minutes\nsomeone-elses-container\tUp 1 hour\n".into(),
        stderr: String::new(),
    });
    let running = launcher(runner.clone()).running().expect("lists");

    // The docker query must filter by our label, so an unrelated container is
    // never a candidate for us to stop.
    assert!(
        runner.calls()[0]
            .iter()
            .any(|a| a.contains("io.atlasctl.managed=1"))
    );

    assert_eq!(
        running[0].recipe.as_ref().map(|r| r.as_str()),
        Some("qwen3.6-27b-fp8")
    );
    assert_eq!(running[0].status, "Up 3 minutes");
    // A container that is not ours by naming convention still lists, with no
    // recipe attached, rather than being silently dropped.
    assert_eq!(running[1].recipe, None);
}

#[test]
fn docker_being_unavailable_is_reported_as_such() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 1,
        stdout: String::new(),
        stderr: "cannot connect to the docker daemon".into(),
    });
    let err = launcher(runner).running().expect_err("must fail");
    assert!(
        matches!(err, AgentError::DockerUnavailable { .. }),
        "{err:?}"
    );
}

/// A model that is not in the cache is refused BEFORE docker is invoked.
///
/// The launch runs offline, so an absent model cannot be fetched; without this
/// the container starts, fails inside on the Hub library's own cache-miss, and
/// exits — and a browser operator sees only that it stopped. The assertion that
/// the runner recorded nothing is the point: refusing late would still "fail",
/// just uselessly.
#[test]
fn a_model_missing_from_the_cache_is_refused_without_running_docker() {
    let runner = Arc::new(RecordingRunner::new());
    let mut h = host();
    h.hf_cache_dir = std::env::temp_dir()
        .join(format!("atlasctl-empty-cache-{}", std::process::id()))
        .display()
        .to_string();
    let l = DockerLauncher::new(runner.clone(), h, &ROOTLESS_V1, Box::new(NvidiaDevices));

    let err = l
        .launch(&recipe(), &BTreeMap::new())
        .expect_err("an absent model must be refused");
    let AgentError::LaunchFailed { detail } = err else {
        panic!("expected LaunchFailed, got {err:?}");
    };
    assert!(detail.contains("org/m"), "must name the model: {detail}");
    assert!(
        detail.contains("hf download"),
        "must say how to fix it: {detail}"
    );
    assert!(
        runner.calls().is_empty(),
        "docker must not be invoked at all, got {:?}",
        runner.calls()
    );
}

/// A cache directory that exists but holds no weights is refused, and says so.
///
/// `Path::exists` on the model directory is not enough: an interrupted or
/// metadata-only download leaves `refs/`, `snapshots/` and a `config.json`
/// behind. Telling that operator the model is "not there" when they can see the
/// directory reads as a broken tool, so the message names the real state.
#[test]
fn a_metadata_only_cache_entry_is_refused_and_named_as_such() {
    let runner = Arc::new(RecordingRunner::new());
    let root = std::env::temp_dir().join(format!("atlasctl-meta-cache-{}", std::process::id()));
    let snap = root.join("hub/models--org--m/snapshots/deadbeef");
    std::fs::create_dir_all(&snap).expect("mkdir");
    std::fs::write(snap.join("config.json"), b"{}").expect("write config");
    // Deliberately no weight file.

    let mut h = host();
    h.hf_cache_dir = root.display().to_string();
    let l = DockerLauncher::new(runner.clone(), h, &ROOTLESS_V1, Box::new(NvidiaDevices));

    let err = l
        .launch(&recipe(), &BTreeMap::new())
        .expect_err("a weightless cache entry must be refused");
    let AgentError::LaunchFailed { detail } = err else {
        panic!("expected LaunchFailed, got {err:?}");
    };
    assert!(
        detail.contains("no weight files"),
        "must name the real state rather than claiming absence: {detail}"
    );
    assert!(runner.calls().is_empty(), "docker must not be invoked");

    std::fs::remove_dir_all(&root).ok();
}

/// A recipe that brings its own weights is NOT refused for a cache miss.
///
/// `--model-from-path` points the engine at a directory, so the Hub cache is
/// never consulted and a "missing model" refusal would block a launch that was
/// going to work. That escape hatch is the whole reason the guard reads the
/// rendered argv rather than the model field, and nothing tested it: the guard
/// could have been tightened later — or the flag renamed — and the only symptom
/// would be a refusal nobody could explain.
#[test]
fn a_recipe_that_brings_its_own_weights_is_not_refused() {
    let runner = Arc::new(RecordingRunner::new());
    // A cache with nothing in it at all, so the guard would certainly fire.
    let mut h = host();
    h.hf_cache_dir = std::env::temp_dir()
        .join(format!("atlasctl-byo-weights-{}", std::process::id()))
        .display()
        .to_string();
    let l = DockerLauncher::new(runner.clone(), h, &ROOTLESS_V1, Box::new(NvidiaDevices));

    let r = Recipe::parse(
        "r",
        concat!(
            "model: org/m\n",
            "container: img:tag\n",
            "runtime: atlas\n",
            "defaults:\n",
            "  port: 8888\n",
            "  model_from_path: /models/local\n",
        ),
        Provenance::Builtin {
            path: "r.yaml".into(),
        },
    )
    .expect("fixture parses");

    let started = l.launch(&r, &BTreeMap::new()).expect("must not be refused");
    assert_eq!(started.container, "atlas-r");
    // The guard is skipped by reading the RENDERED command, so prove the flag
    // actually reached it — a recipe whose setting was silently dropped would
    // pass this test for the wrong reason.
    let ran = runner.calls().concat().join(" ");
    assert!(
        ran.contains("--model-from-path"),
        "the flag must reach the command: {ran}"
    );
}
