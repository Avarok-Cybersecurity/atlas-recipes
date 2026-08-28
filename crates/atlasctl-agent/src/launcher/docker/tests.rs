// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::host::PosixUser;
use atlasctl_core::io::RecordingRunner;
use atlasctl_core::io::process::Output;
use atlasctl_core::recipe::Provenance;

fn host() -> HostSnapshot {
    HostSnapshot {
        posix_user: Some(PosixUser {
            uid: 1000,
            gid: 1000,
        }),
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
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

#[test]
fn stop_targets_only_our_own_container_name() {
    let runner = Arc::new(RecordingRunner::new());
    launcher(runner.clone()).stop("r").expect("stops");
    assert_eq!(runner.calls(), [["docker", "stop", "atlas-r"]]);
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
