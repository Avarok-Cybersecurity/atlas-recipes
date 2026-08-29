// SPDX-License-Identifier: AGPL-3.0-only

//! The commit half of the rank-service tests, split from `tests.rs` for size.
//! The helpers (`service`, `agreeing`, `RecordingRunner`) stay in that module
//! and are used from here.

use super::tests::{RECIPE, agreeing, host, node, service};
use super::*;
use atlasctl_core::docker::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::io::RecordingRunner;
use atlasctl_core::io::process::Output;
use std::sync::Arc;

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

/// The failure that hung a real two-node launch.
///
/// A DGX Spark carries several point-to-point RoCE links. The head offered its
/// address on one this machine is not attached to, and the rank started, waited
/// at the collective barrier, and retried `Connection timed out` every second
/// while the operator saw two healthy containers and no server.
///
/// It is a refusal now, at prepare, before anything starts.
#[test]
fn a_rendezvous_address_on_a_link_this_node_lacks_is_refused() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let mut a = agreeing(&svc, 1);
    // The fixture node is on 10.0.0.2/30; this is the head's *other* link.
    a.master_addr = "10.10.10.13".to_owned();

    let PrepareReply::Refused { reason } = svc.prepare("e1", &a) else {
        panic!("an unreachable rendezvous must be refused, not reserved");
    };
    assert!(reason.contains("10.10.10.13"), "{reason}");
    assert!(
        reason.contains("10.0.0.2/30"),
        "must name this node's links: {reason}"
    );
    assert!(
        svc.commit("e1").is_err(),
        "a refused prepare must reserve nothing"
    );
}

/// The address on the shared link still works — the check must not refuse
/// everything.
#[test]
fn a_rendezvous_address_on_the_shared_link_is_accepted() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let mut a = agreeing(&svc, 1);
    a.master_addr = "10.0.0.1".to_owned();
    assert_eq!(svc.prepare("e1", &a), PrepareReply::Prepared);
}

/// The collective has to be told which link to use, or it picks one itself.
///
/// Nothing set NCCL_SOCKET_IFNAME or NCCL_IB_HCA, on the reasoning that
/// guessing a NIC name silently uses the wrong fabric. True of guessing — but
/// leaving them unset delegates the guess to NCCL, which on a machine with four
/// RoCE ports chose the one that reaches nobody and died at `ibv_modify_qp`
/// with `Connection timed out`, then took the process with it via
/// `CUDA_ERROR_ILLEGAL_ADDRESS`.
mod collective_binding {
    use super::*;

    #[test]
    fn a_rank_pins_the_collective_to_the_rendezvous_link() {
        let runner = Arc::new(RecordingRunner::new());
        let svc = service(&runner, Ok(()));
        let mut a = agreeing(&svc, 1);
        a.master_addr = "10.0.0.1".to_owned(); // the fixture's shared /30

        let (cmd, _) = svc.render(&a).expect("renders");
        assert!(
            cmd.contains("NCCL_SOCKET_IFNAME=enp1s0f0np0"),
            "must name the interface carrying the rendezvous: {cmd}"
        );
        assert!(
            cmd.contains("NCCL_IB_HCA=rocep1s0f0"),
            "must name that interface's own RDMA device: {cmd}"
        );
        assert!(
            !cmd.contains("rocep1s0f1"),
            "the other port must not appear: {cmd}"
        );
    }

    /// A solo launch has no collective, so pinning one would be noise that
    /// also constrains a single-node server for no reason.
    #[test]
    fn a_solo_launch_pins_nothing() {
        let recipe = atlasctl_core::registry::RegistrySet::builtin_only()
            .resolve(&atlasctl_core::registry::RecipeRef::parse(
                "qwen3.6-35b-a3b-fp8-bf16head",
            ))
            .expect("a shipped solo recipe");
        let plan = atlasctl_core::docker::translate::translate(
            &recipe,
            &BTreeMap::new(),
            &atlasctl_core::chain::UserConfig::default(),
            &host(),
            &atlasctl_core::docker::translate::Placement::Solo,
            &atlasctl_core::docker::translate::LaunchContext {
                profile: &ROOTLESS_V1,
                devices: &NvidiaDevices,
                collective: &atlasctl_core::docker::collective::NcclRoce,
            },
        )
        .expect("translates");
        assert!(!plan.docker.env.contains_key("NCCL_SOCKET_IFNAME"));
        assert!(!plan.docker.env.contains_key("NCCL_IB_HCA"));
    }

    /// A routed rendezvous has no single local interface to name, and inventing
    /// one would be exactly the guess this avoids.
    #[test]
    fn a_rendezvous_off_every_local_subnet_pins_nothing() {
        let runner = Arc::new(RecordingRunner::new());
        let svc = service(&runner, Ok(()));
        let mut a = agreeing(&svc, 1);
        a.master_addr = "203.0.113.7".to_owned();

        let (cmd, _) = svc.render(&a).expect("renders");
        assert!(!cmd.contains("NCCL_SOCKET_IFNAME"), "{cmd}");
        assert!(!cmd.contains("NCCL_IB_HCA"), "{cmd}");
    }
}

/// `stop` runs `docker rm -f` on a name that arrives from whichever node is
/// acting as head. Unbounded, that is a peer holding the `controller` grant
/// force-removing ANY container on this machine — well past "drive its own
/// launch, one hop". The browser's own stop has always been scoped to
/// `atlas-{recipe}`; this path had no equivalent.
#[test]
fn a_rank_may_only_stop_a_container_this_fleet_launched() {
    use crate::rankservice::is_ours;
    for foreign in [
        "postgres",
        "someone-elses-db",
        "ATLAS-shouty",
        "atlas-",
        "atlas-../evil",
        "",
    ] {
        assert!(!is_ours(foreign), "{foreign:?} was not launched by us");
    }
}

/// And the names this fleet really produces must still be stoppable, or a
/// cluster launch could never be torn down.
#[test]
fn the_names_translate_actually_produces_are_still_ours() {
    use crate::rankservice::is_ours;
    for mine in [
        "atlas-qwen3.6-27b-nvfp4-unsloth",
        "atlas-qwen3.6-35b-a3b-fp8-bf16head-rank0",
        "atlas-qwen3.6-35b-a3b-fp8-bf16head-rank11",
    ] {
        assert!(is_ours(mine), "{mine:?} is a name translate produces");
    }
}
