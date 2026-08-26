// SPDX-License-Identifier: AGPL-3.0-only

//! The reservation, which is what makes commit safe to carry only an epoch.

use super::*;
use atlasctl_core::docker::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::io::RecordingRunner;
use atlasctl_core::io::process::Output;
use atlasctl_protocol::fleet::NodeId;
use std::collections::BTreeMap;

/// A real two-node recipe, so the assignment being rendered is one that
/// actually exists rather than one invented for the test.
const RECIPE: &str = "qwen3.5-122b-a10b-nvfp4-ep2";

fn host() -> atlasctl_core::host::HostSnapshot {
    atlasctl_core::host::HostSnapshot {
        uid: 1000,
        gid: 1000,
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
        env: BTreeMap::new(),
    }
}

fn node() -> NodeId {
    NodeId::parse(&"ab".repeat(32)).expect("64 hex chars")
}

fn service(runner: &Arc<RecordingRunner>, can_launch: Result<(), String>) -> LocalRankService {
    LocalRankService::new(
        atlasctl_core::registry::RegistrySet::builtin_only(),
        host(),
        &ROOTLESS_V1,
        Box::new(NvidiaDevices),
        Box::new(atlasctl_core::docker::collective::NcclRoce),
        Arc::clone(runner) as Arc<dyn ProcessRunner>,
        can_launch,
    )
}

/// An assignment carrying the hash this machine itself computes.
fn agreeing(svc: &LocalRankService, rank: u16) -> RankAssignment {
    RankAssignment {
        node: node(),
        rank,
        world_size: 2,
        master_addr: "10.0.0.1".to_owned(),
        master_port: 29500,
        recipe: RECIPE.to_owned(),
        recipe_hash: svc.content_hash(RECIPE).expect("a shipped recipe"),
        settings: BTreeMap::new(),
    }
}

fn docker_run(runner: &RecordingRunner) -> Option<Vec<String>> {
    runner.calls().into_iter().find(|c| {
        c.first().map(String::as_str) == Some("docker")
            && c.get(1).map(String::as_str) == Some("run")
    })
}

/// Preview and execution must come from the same rendering, or the preview is
/// decoration.
#[test]
fn a_prepared_rank_commits_the_command_it_rendered() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);

    let (rendered, _) = svc.render(&a).expect("renders");
    assert_eq!(svc.prepare("e1", &a), PrepareReply::Prepared);
    svc.commit("e1").expect("commits");

    let ran = docker_run(&runner).expect("docker run was called");
    for token in ["--ipc=host", "--network=host"] {
        assert!(
            ran.contains(&token.to_owned()),
            "{token} missing from {ran:?}"
        );
        assert!(rendered.contains(token), "{token} missing from the preview");
    }
}

/// Two nodes running different revisions of one recipe would launch two
/// different models and call it one cluster; the failure would surface as wrong
/// output rather than as an error.
#[test]
fn a_recipe_the_head_hashes_differently_is_refused() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let mut a = agreeing(&svc, 1);
    a.recipe_hash = "0".repeat(64);

    let PrepareReply::Refused { reason } = svc.prepare("e1", &a) else {
        panic!("a revision mismatch must be refused");
    };
    assert!(reason.contains("differs between nodes"), "{reason}");
    assert!(
        svc.commit("e1").is_err(),
        "a refused prepare reserves nothing"
    );
}

/// An empty hash is not agreement: a head that stated nothing must not be
/// treated as a head that stated the right thing.
#[test]
fn a_head_that_states_no_revision_is_refused() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let mut a = agreeing(&svc, 1);
    a.recipe_hash = String::new();

    let PrepareReply::Refused { reason } = svc.prepare("e1", &a) else {
        panic!("an unstated revision must be refused");
    };
    assert!(reason.contains("(none sent)"), "{reason}");
}

/// A machine that has agreed to be rank 1 of one cluster must refuse to be rank
/// 1 of another, or the second commit would replace the first mid-launch.
#[test]
fn a_second_clusters_prepare_is_refused_while_one_is_held() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);

    assert_eq!(svc.prepare("first", &a), PrepareReply::Prepared);
    let PrepareReply::Refused { reason } = svc.prepare("second", &a) else {
        panic!("a held reservation must block another cluster");
    };
    // "already running" would send an operator looking for a container that
    // does not exist; what they need to do is abandon the other launch.
    assert!(reason.contains("already reserved"), "{reason}");
    assert!(reason.contains("abort that launch"), "{reason}");
}

/// A retried prepare after a dropped connection is ordinary, not a second
/// cluster.
#[test]
fn re_preparing_the_same_epoch_is_allowed() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);

    assert_eq!(svc.prepare("e1", &a), PrepareReply::Prepared);
    assert_eq!(svc.prepare("e1", &a), PrepareReply::Prepared);
    assert!(svc.commit("e1").is_ok());
}

#[test]
fn a_commit_without_a_prepare_starts_nothing() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));

    let err = svc.commit("never-prepared").expect_err("nothing reserved");
    assert!(err.to_string().contains("prepare first"), "{err}");
    assert_eq!(runner.call_count(), 0, "no process may be run");
}

#[test]
fn a_commit_quoting_another_epoch_starts_nothing() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);
    svc.prepare("e1", &a);
    let before = runner.call_count();

    let err = svc.commit("e2").expect_err("wrong epoch");
    assert!(err.to_string().contains("not e2"), "{err}");
    assert_eq!(runner.call_count(), before, "nothing may be started");
}

/// A reservation is spent by its commit, so a replayed frame cannot start a
/// second container on a machine already running the first.
#[test]
fn a_replayed_commit_starts_nothing_twice() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);
    svc.prepare("e1", &a);
    svc.commit("e1").expect("first commit");

    let before = runner.call_count();
    assert!(svc.commit("e1").is_err(), "the reservation is spent");
    assert_eq!(runner.call_count(), before, "nothing may run again");
}

/// An abort for a stale epoch arriving late must not release a reservation made
/// since — that would silently cancel a launch the operator is watching.
#[test]
fn a_stale_abort_does_not_release_a_newer_reservation() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);

    svc.prepare("old", &a);
    svc.abort("old");
    svc.prepare("new", &a);
    svc.abort("old");

    assert!(
        svc.commit("new").is_ok(),
        "the newer reservation must survive"
    );
}

/// Rollback asking about a container that never started is an ordinary race,
/// not a failure to show instead of the real reason.
#[test]
fn stopping_a_container_that_is_already_gone_succeeds() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 1,
        stdout: String::new(),
        stderr: "Error response from daemon: No such container: atlas-x".to_owned(),
    });
    let svc = service(&runner, Ok(()));
    svc.stop("atlas-x")
        .expect("already gone is the wanted outcome");
}

#[test]
fn a_container_runtime_failure_on_stop_is_reported() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 1,
        stdout: String::new(),
        stderr: "permission denied while trying to connect to the Docker daemon".to_owned(),
    });
    let svc = service(&runner, Ok(()));
    let err = svc
        .stop("atlas-x")
        .expect_err("a real failure must surface");
    assert!(err.to_string().contains("permission denied"), "{err}");
}

/// A machine that cannot run models says so before rendering anything, rather
/// than reserving and failing at commit.
#[test]
fn a_client_only_agent_refuses_to_be_a_rank() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Err("this agent runs in --client mode".to_owned()));
    let a = RankAssignment {
        node: node(),
        rank: 1,
        world_size: 2,
        master_addr: "10.0.0.1".to_owned(),
        master_port: 29500,
        recipe: RECIPE.to_owned(),
        recipe_hash: svc.content_hash(RECIPE).expect("a shipped recipe"),
        settings: BTreeMap::new(),
    };

    let PrepareReply::Refused { reason } = svc.prepare("e1", &a) else {
        panic!("a control-only node must refuse");
    };
    assert!(reason.contains("--client mode"), "{reason}");
    assert_eq!(runner.call_count(), 0, "it must not even probe docker");
}

/// `docker run -d` returning 0 means the container was created, not that the
/// workload survived. This is the question that catches the difference.
#[test]
fn a_running_container_reports_alive() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 0,
        stdout: "true\n".to_owned(),
        stderr: String::new(),
    });
    let svc = service(&runner, Ok(()));
    assert!(svc.alive("atlas-x").expect("asks"));

    let asked = runner.calls().into_iter().next().expect("one call");
    assert_eq!(asked[0], "docker");
    assert_eq!(asked[1], "inspect");
    assert_eq!(asked.last().expect("the container"), "atlas-x");
}

#[test]
fn a_stopped_container_reports_not_alive() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 0,
        stdout: "false\n".to_owned(),
        stderr: String::new(),
    });
    let svc = service(&runner, Ok(()));
    assert!(!svc.alive("atlas-x").expect("asks"));
}

/// A rank that died under `--rm` has already been removed, so `docker inspect`
/// fails. That is the exact case this exists to catch, and it is an answer
/// rather than an error.
#[test]
fn a_container_removed_after_dying_reports_not_alive() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(Output {
        status: 1,
        stdout: String::new(),
        stderr: "Error: No such object: atlas-x".to_owned(),
    });
    let svc = service(&runner, Ok(()));
    assert!(
        !svc.alive("atlas-x")
            .expect("a missing container is an answer")
    );
}
