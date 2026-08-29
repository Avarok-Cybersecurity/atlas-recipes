// SPDX-License-Identifier: AGPL-3.0-only

//! Taking a cluster down, and noticing when one takes itself down.
//!
//! Split from [`super::cases`] for size. These are the orderings after
//! something is already running: a deliberate stop, a rank that dies on
//! startup, and a rank that dies once the operator has been told it is up.

use super::tests::*;
use super::*;

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
    assert!(
        c.contains(&"local.stop(head-container)".to_owned()),
        "{c:?}"
    );
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
    assert_eq!(
        calls(&log).len(),
        before,
        "no machine may be contacted again"
    );
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
    assert!(
        err.contains("spark-1"),
        "the dead machine must be named: {err}"
    );

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
    assert!(
        c.contains(&"local.alive(head-container)".to_owned()),
        "{c:?}"
    );
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

/// The gap the settle gate cannot close.
///
/// A rank that dies four minutes in — during model build — passed the
/// five-second gate. Its peers then hold their GPUs indefinitely, waiting at a
/// rendezvous that will never complete, and nothing notices.
#[test]
fn a_rank_that_dies_after_commit_tears_the_cluster_down() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("commits");

    // It was whole a moment ago.
    assert!(d.supervise().is_none(), "a healthy cluster is left alone");

    // Now rank 2 dies.
    d.kill_for_test(node_id(2));
    let torn = d.supervise().expect("a dead rank must be noticed");
    assert!(
        torn.why.contains("spark-2"),
        "must name what died: {}",
        torn.why
    );
    // The machines travel with the sentence, so the caller can raise this
    // against a node rather than only printing it on the head's stderr.
    assert!(
        !torn.nodes.is_empty(),
        "supervision must say WHICH machine, not only that something died"
    );

    let c = calls(&log);
    assert!(
        c.contains(&"local.stop(head-container)".to_owned()),
        "the survivors must be stopped: {c:?}"
    );
    assert!(
        d.stop_cluster().is_err(),
        "the cluster is gone, so there is nothing left to stop"
    );
}

/// A second cluster must be refused while one is running.
///
/// Nothing used to stop it, and committing was destructive in both directions:
/// with the SAME recipe each rank's `docker rm -f` silently killed the live
/// cluster mid-service, and with a different one both ran, contending for the
/// GPUs, while the second commit overwrote the record of the first -- leaving
/// the original with no Stop button that knew about it.
#[test]
fn a_second_cluster_is_refused_while_one_is_running() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect("prepares");
    d.commit(&epoch).expect("commits");

    let before = calls(&log).len();
    let err = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .expect_err("a second cluster must be refused while one runs");
    assert!(err.contains("already running"), "{err}");
    assert!(err.contains("stop it"), "must say what to do: {err}");
    // And it must refuse BEFORE touching a machine: a refusal that has already
    // dialled three peers has already done the damage it was avoiding.
    assert_eq!(
        calls(&log).len(),
        before,
        "the refusal must not reach any machine"
    );
}

/// Supervising when nothing is running must not reach for the network.
#[test]
fn supervising_an_idle_agent_does_nothing() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    assert!(d.supervise().is_none());
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));
}
