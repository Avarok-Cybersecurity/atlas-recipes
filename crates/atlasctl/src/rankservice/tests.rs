// SPDX-License-Identifier: AGPL-3.0-only

//! The reservation, which is what makes commit safe to carry only an epoch.

use super::*;
use atlasctl_core::docker::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::host::PosixUser;
use atlasctl_core::io::RecordingRunner;
use atlasctl_protocol::fleet::NodeId;
use std::collections::BTreeMap;

/// A real two-node recipe, so the assignment being rendered is one that
/// actually exists rather than one invented for the test.
pub(super) const RECIPE: &str = "qwen3.5-122b-a10b-nvfp4-ep2";

pub(super) fn host() -> atlasctl_core::host::HostSnapshot {
    atlasctl_core::host::HostSnapshot {
        posix_user: Some(PosixUser {
            uid: 1000,
            gid: 1000,
        }),
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
        env: BTreeMap::new(),
    }
}

/// A network in which nothing is routable beyond the local links.
struct NeverAnswers;

impl atlasctl_agent::rendezvous::Reachability for NeverAnswers {
    fn answers(&self, _: &str, _: u16) -> bool {
        false
    }
}

/// This machine's own link, sharing a /30 with the fixture rendezvous address.
fn local_link() -> atlasctl_protocol::fleet::NodeAddress {
    atlasctl_protocol::fleet::NodeAddress {
        iface: "enp1s0f0np0".to_owned(),
        addr: "10.0.0.2".to_owned(),
        class: atlasctl_protocol::fleet::LinkClass::Roce,
        speed_mbps: Some(200_000),
        rdma: true,
        prefix_len: 30,
    }
}

pub(super) fn node() -> NodeId {
    NodeId::parse(&"ab".repeat(32)).expect("64 hex chars")
}

pub(super) fn service(
    runner: &Arc<RecordingRunner>,
    can_launch: Result<(), String>,
) -> LocalRankService {
    LocalRankService::new(
        atlasctl_core::registry::RegistrySet::builtin_only(),
        host(),
        &ROOTLESS_V1,
        Box::new(NvidiaDevices),
        Box::new(atlasctl_core::docker::collective::NcclRoce),
        Arc::clone(runner) as Arc<dyn ProcessRunner>,
        RankEnvironment {
            can_launch,
            // On the same /30 as the fixtures' rendezvous address, which is
            // what a real head would offer: an address on a link this node is
            // attached to.
            local_addresses: vec![local_link()],
            // Nothing answers. The address that broke the real cluster is one
            // of this host's own interfaces, so a live probe would report it
            // reachable and the refusal test would pass for the wrong reason.
            reachability: Box::new(NeverAnswers),
            // The real pair's mapping: each RoCE port has its own device.
            rdma_devices: [
                ("enp1s0f0np0".to_owned(), "rocep1s0f0".to_owned()),
                ("enp1s0f1np1".to_owned(), "rocep1s0f1".to_owned()),
            ]
            .into_iter()
            .collect(),
        },
    )
}

/// An assignment carrying the hash this machine itself computes.
pub(super) fn agreeing(svc: &LocalRankService, rank: u16) -> RankAssignment {
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

/// A reservation whose head went away must not hold this machine forever.
///
/// The failure it prevents: an operator prepares, then the tab closes or the
/// head agent restarts before commit or abort. Nothing on this machine ever
/// released the hold, so every later cluster launch was refused -- and the
/// refusal said "abort that launch, or wait for it to finish", when abort
/// needs the epoch that went away with the head and it never finished. On a
/// fleet that bricks every machine at once until each agent is restarted by
/// hand.
#[test]
fn a_reservation_whose_head_vanished_lapses_and_the_refusal_says_when() {
    let runner = Arc::new(RecordingRunner::new());
    let svc = service(&runner, Ok(()));
    let a = agreeing(&svc, 1);

    assert_eq!(svc.prepare("abandoned", &a), PrepareReply::Prepared);

    // While it is fresh, another cluster is still refused -- and now the
    // refusal states how long waiting would take, which is the only reason
    // telling someone to wait is fair.
    let PrepareReply::Refused { reason } = svc.prepare("other", &a) else {
        panic!("a fresh reservation must still block another cluster");
    };
    assert!(reason.contains("already reserved"), "{reason}");
    assert!(
        reason.contains("lapse"),
        "the refusal must say waiting works: {reason}"
    );

    // Age it past the TTL, exactly as an abandoned head would. The deadline is
    // pulled BACK to now rather than pushing the creation time into the past:
    // subtracting a ten-minute Duration from `Instant::now()` panics on Windows
    // whenever the machine booted more recently than that -- which a CI runner
    // always has. That panic is how this test broke main after passing here.
    {
        let mut held = svc.reserved.lock().expect("reservation lock poisoned");
        let r = held.as_mut().expect("still reserved");
        r.expires = std::time::Instant::now();
    }
    assert_eq!(
        svc.prepare("other", &a),
        PrepareReply::Prepared,
        "a lapsed reservation must not keep refusing new clusters"
    );
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
