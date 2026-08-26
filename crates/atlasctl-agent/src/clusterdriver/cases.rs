// SPDX-License-Identifier: AGPL-3.0-only

//! The orderings that produce a partial cluster.

use super::tests::*;
use super::*;

#[test]
fn a_prepare_every_rank_accepts_may_commit() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("the plan is possible");

    assert!(may, "every rank accepted");
    assert_eq!(ranks.len(), 3);
    assert!(ranks.iter().all(|r| r.prepared));
    assert!(!epoch.is_empty());
    // Nothing started: prepare reserves, it does not launch.
    assert!(
        !calls(&log).iter().any(|c| c.contains("commit")),
        "prepare must not start anything: {:?}",
        calls(&log)
    );
}

/// The reason two phases exist. One machine says no, and every machine that
/// already said yes is still holding a reservation — if those are not released,
/// the fleet cannot launch anything until each is prepared again.
#[test]
fn one_refusal_releases_every_reservation_already_taken() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).refusing(node_id(3), "no disk"),
    );
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("the plan is possible");

    assert!(!may, "a refusal must block the commit");
    let refused: Vec<_> = ranks.iter().filter(|r| !r.prepared).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].reason, "no disk");

    let c = calls(&log);
    // The two that accepted were released; the one that refused was not asked
    // to release something it never took.
    assert!(c.contains(&"local.abort".to_owned()), "head must release: {c:?}");
    assert!(
        c.contains(&format!("{}.abort", node_id(2).short())),
        "the peer that accepted must release: {c:?}"
    );
    assert!(
        !c.contains(&format!("{}.abort", node_id(3).short())),
        "the refuser holds nothing to release: {c:?}"
    );
    assert!(
        !c.iter().any(|x| x.contains("commit")),
        "nothing may start: {c:?}"
    );
}

/// A machine that cannot be reached has not agreed to anything, so it must read
/// as a refusal rather than abandon the ranks before it mid-loop.
#[test]
fn an_unreachable_rank_is_a_refusal_not_an_abandoned_launch() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).refusing(node_id(2), "could not be reached: timed out"),
    );
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("the plan is possible");

    assert!(!may);
    // Every rank still got an answer — the loop did not stop at the failure.
    assert_eq!(ranks.len(), 3);
    assert!(calls(&log).contains(&"local.abort".to_owned()));
}

#[test]
fn a_commit_starts_every_rank_and_only_rank_zero_gets_an_endpoint() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    assert!(may);

    let started = d.commit(&epoch).expect("commits");
    assert_eq!(started.len(), 3);
    assert_eq!(
        started.iter().filter(|r| r.endpoint.is_some()).count(),
        1,
        "a worker's URL would not answer, so it must not be offered"
    );
    assert!(started[0].rank == 0 && started[0].endpoint.is_some());
}

/// A half-started cluster waits forever on a rendezvous that never completes,
/// and the operator sees a hang rather than an error.
#[test]
fn a_failed_commit_stops_the_ranks_that_already_started() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).failing_commit(node_id(3), "image missing"),
    );
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");

    let err = d.commit(&epoch).expect_err("rank 2 cannot start");
    assert!(err.contains("image missing"), "the reason must survive: {err}");

    let c = calls(&log);
    assert!(
        c.contains(&"local.stop(head-container)".to_owned()),
        "rank 0 must be stopped: {c:?}"
    );
    assert!(
        c.iter().any(|x| x.starts_with(&format!("{}.stop(", node_id(2).short()))),
        "the started peer must be stopped: {c:?}"
    );
}

/// A commit consumes its prepare. Replaying the frame must not start a second
/// cluster on machines already running the first.
#[test]
fn a_replayed_commit_starts_nothing_twice() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("first commit succeeds");

    let before = calls(&log).len();
    let err = d.commit(&epoch).expect_err("the prepare is spent");
    assert!(err.contains("no prepare is outstanding"), "{err}");
    assert_eq!(
        calls(&log).len(),
        before,
        "a replayed commit must not reach a single rank"
    );
}

/// An epoch from another attempt cannot authorize this one.
#[test]
fn a_commit_quoting_the_wrong_epoch_is_refused() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (_, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");

    let before = calls(&log).len();
    let err = d.commit("some-other-epoch").expect_err("wrong epoch");
    assert!(err.contains("is holding a prepare for"), "{err}");
    assert_eq!(calls(&log).len(), before, "no rank may be asked anything");
}

#[test]
fn an_abort_releases_every_rank() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");

    d.abort(&epoch);
    let c = calls(&log);
    assert!(c.contains(&"local.abort".to_owned()));
    assert!(c.contains(&format!("{}.abort", node_id(2).short())));
    assert!(c.contains(&format!("{}.abort", node_id(3).short())));

    // And the prepare is gone, so it cannot then be committed.
    assert!(d.commit(&epoch).is_err());
}

/// An abort for a stale epoch arriving late must not release the prepare made
/// since — that would silently cancel a launch the operator is watching.
#[test]
fn a_stale_abort_does_not_release_a_newer_prepare() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (first, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    let (second, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares again");
    assert_ne!(first, second, "each attempt gets its own epoch");

    d.abort(&first);
    assert!(
        d.commit(&second).is_ok(),
        "the newer prepare must survive a stale abort"
    );
}

/// The selection is not a suggestion: a machine the fleet does not know about
/// must fail before anything is asked of anybody.
#[test]
fn a_machine_outside_the_fleet_fails_before_any_rank_is_asked() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let err = d
        .prepare(
            &recipe(),
            &[node_id(1), node_id(9)],
            node_id(1),
            &BTreeMap::new(),
        )
        .expect_err("node 9 is not in this fleet");
    assert!(err.contains("not in this fleet"), "{err}");
    assert!(calls(&log).is_empty(), "nothing may be asked: {:?}", calls(&log));
}

/// The head is a rank like any other. A head that skipped its own prepare would
/// commit a rank nobody validated.
#[test]
fn the_head_prepares_itself_like_any_other_rank() {
    let log = new_log();
    let (d, _) = driver(refusing_rank(&log, "docker is down"), transport(&log));
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("the plan is possible");

    assert!(!may, "the head refusing must block the commit too");
    let head = ranks.iter().find(|r| r.rank == 0).expect("rank 0");
    assert!(!head.prepared);
    assert_eq!(head.reason, "docker is down");
}

#[test]
fn a_started_cluster_can_be_stopped_on_every_machine() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    let started = d.commit(&epoch).expect("commits");

    let stopped = d.stop_cluster().expect("stops");
    assert_eq!(stopped.len(), started.len());

    let c = calls(&log);
    assert!(c.contains(&"local.stop(head-container)".to_owned()), "{c:?}");
    for seed in [2u8, 3] {
        let id = node_id(seed).short();
        assert!(
            c.iter().any(|x| x.starts_with(&format!("{id}.stop("))),
            "{id} was never stopped: {c:?}"
        );
    }
}

/// An agent stops the cluster it started. Asking it to stop one it did not is
/// how a page would use its local agent to reach machines it cannot authorize.
#[test]
fn stopping_without_having_started_anything_is_refused() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let err = d.stop_cluster().expect_err("nothing was started");
    assert!(err.contains("did not start a cluster"), "{err}");
    assert!(calls(&log).is_empty(), "no machine may be contacted");
}

/// A cluster is stopped once. A second stop must not tear down a cluster
/// started since.
#[test]
fn a_cluster_is_stopped_once() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("commits");
    d.stop_cluster().expect("stops");

    let before = calls(&log).len();
    assert!(d.stop_cluster().is_err(), "the cluster is already stopped");
    assert_eq!(calls(&log).len(), before, "no machine may be contacted again");
}

/// A rank left running holds a whole GPU, so giving up on the first failure
/// would be the most expensive possible response to it.
#[test]
fn every_rank_is_attempted_even_when_one_refuses_to_stop() {
    let log = new_log();
    let (d, _) = driver(refusing_stop_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("commits");

    let err = d.stop_cluster().expect_err("rank 0 could not be stopped");
    assert!(err.contains("could not stop"), "{err}");

    let c = calls(&log);
    for seed in [2u8, 3] {
        let id = node_id(seed).short();
        assert!(
            c.iter().any(|x| x.starts_with(&format!("{id}.stop("))),
            "{id} must still be stopped: {c:?}"
        );
    }
}

/// The failure that real hardware found. `docker run -d` returned 0 for rank 0,
/// so the commit reported success — and rank 0's container had already died a
/// second later, leaving rank 1 running alone and waiting forever on a
/// rendezvous that would never complete. The operator saw a hang, not an error.
#[test]
fn a_rank_that_dies_on_startup_fails_the_commit_and_stops_the_rest() {
    let log = new_log();
    let (d, _) = driver(dying_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");

    let err = d.commit(&epoch).expect_err("rank 0 did not survive");
    assert!(err.contains("stopped within"), "{err}");
    assert!(err.contains("spark-1"), "the dead machine must be named: {err}");

    // The peers that did start must not be left holding a GPU.
    let c = calls(&log);
    for seed in [2u8, 3] {
        let id = node_id(seed).short();
        assert!(
            c.iter().any(|x| x.starts_with(&format!("{id}.stop("))),
            "{id} must be stopped: {c:?}"
        );
    }
}

#[test]
fn a_peer_that_dies_on_startup_fails_the_commit_too() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log).dying(node_id(3)));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");

    let err = d.commit(&epoch).expect_err("rank 2 did not survive");
    assert!(err.contains("spark-3"), "{err}");
    assert!(
        calls(&log).contains(&"local.stop(head-container)".to_owned()),
        "rank 0 must be stopped too: {:?}",
        calls(&log)
    );
}

/// Every rank is checked, not just the first — a cluster is whole or absent.
#[test]
fn every_rank_is_asked_whether_it_survived() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("commits");

    let c = calls(&log);
    assert!(c.contains(&"local.alive(head-container)".to_owned()), "{c:?}");
    for seed in [2u8, 3] {
        let id = node_id(seed).short();
        assert!(
            c.iter().any(|x| x.starts_with(&format!("{id}.alive("))),
            "{id} was never asked: {c:?}"
        );
    }
}

/// A commit that failed its settling gate must leave no cluster behind to stop.
#[test]
fn a_cluster_that_failed_to_settle_is_not_recorded_as_running() {
    let log = new_log();
    let (d, _) = driver(dying_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    let _ = d.commit(&epoch);

    assert!(
        d.stop_cluster().is_err(),
        "nothing is running, so there is nothing to stop"
    );
}
