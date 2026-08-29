// SPDX-License-Identifier: AGPL-3.0-only

//! The orderings that produce a partial cluster.

use super::tests::*;
use super::*;

#[tokio::test]
async fn a_prepare_every_rank_accepts_may_commit() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
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
#[tokio::test]
async fn one_refusal_releases_every_reservation_already_taken() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).refusing(node_id(3), "no disk"),
    );
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("the plan is possible");

    assert!(!may, "a refusal must block the commit");
    let refused: Vec<_> = ranks.iter().filter(|r| !r.prepared).collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].reason, "no disk");

    let c = calls(&log);
    // The two that accepted were released; the one that refused was not asked
    // to release something it never took.
    assert!(
        c.contains(&"local.abort".to_owned()),
        "head must release: {c:?}"
    );
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
#[tokio::test]
async fn an_unreachable_rank_is_a_refusal_not_an_abandoned_launch() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).refusing(node_id(2), "could not be reached: timed out"),
    );
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("the plan is possible");

    assert!(!may);
    // Every rank still got an answer — the loop did not stop at the failure.
    assert_eq!(ranks.len(), 3);
    assert!(calls(&log).contains(&"local.abort".to_owned()));
}

#[tokio::test]
async fn a_commit_starts_every_rank_and_only_rank_zero_gets_an_endpoint() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");
    assert!(may);

    let started = d.commit(&epoch).await.expect("commits");
    assert_eq!(started.len(), 3);
    assert_eq!(
        started.iter().filter(|r| r.endpoint.is_some()).count(),
        1,
        "a worker's URL would not answer, so it must not be offered"
    );
    assert!(started[0].rank == 0 && started[0].endpoint.is_some());
}

/// `assignment.settings` is a **sparse diff** — overrides only, not the
/// effective settings. Reading it as the whole truth meant an operator who
/// changed nothing produced an empty map, and the endpoint offered `:8000`
/// while the model served on the `:8888` its recipe pins. The URL was the one
/// thing on that screen the operator would actually paste somewhere.
#[tokio::test]
async fn the_endpoint_uses_the_recipes_port_when_the_operator_overrode_nothing() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");
    assert!(may);

    let started = d.commit(&epoch).await.expect("commits");
    let endpoint = started[0].endpoint.as_deref().expect("rank 0 serves");
    assert!(
        endpoint.ends_with(":8888"),
        "the endpoint ignored the recipe's own port: {endpoint}"
    );
}

/// And an override still wins over the recipe, or the setting would do nothing.
#[tokio::test]
async fn an_operators_port_override_still_beats_the_recipe() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let mut settings = BTreeMap::new();
    settings.insert(
        "port".to_owned(),
        atlasctl_protocol::settings::SettingValue::Int(9001),
    );
    let (epoch, _, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &settings)
        .await
        .expect("prepares");
    assert!(may);

    let started = d.commit(&epoch).await.expect("commits");
    let endpoint = started[0].endpoint.as_deref().expect("rank 0 serves");
    assert!(
        endpoint.ends_with(":9001"),
        "the operator's port was ignored: {endpoint}"
    );
}

/// A half-started cluster waits forever on a rendezvous that never completes,
/// and the operator sees a hang rather than an error.
#[tokio::test]
async fn a_failed_commit_stops_the_ranks_that_already_started() {
    let log = new_log();
    let (d, _) = driver(
        ready_rank(&log),
        transport(&log).failing_commit(node_id(3), "image missing"),
    );
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");

    let err = d.commit(&epoch).await.expect_err("rank 2 cannot start");
    assert!(
        err.contains("image missing"),
        "the reason must survive: {err}"
    );

    let c = calls(&log);
    assert!(
        c.contains(&"local.stop(head-container)".to_owned()),
        "rank 0 must be stopped: {c:?}"
    );
    assert!(
        c.iter()
            .any(|x| x.starts_with(&format!("{}.stop(", node_id(2).short()))),
        "the started peer must be stopped: {c:?}"
    );
}

/// A commit consumes its prepare. Replaying the frame must not start a second
/// cluster on machines already running the first.
#[tokio::test]
async fn a_replayed_commit_starts_nothing_twice() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");
    d.commit(&epoch).await.expect("first commit succeeds");

    let before = calls(&log).len();
    let err = d.commit(&epoch).await.expect_err("the prepare is spent");
    assert!(err.contains("no prepare is outstanding"), "{err}");
    assert_eq!(
        calls(&log).len(),
        before,
        "a replayed commit must not reach a single rank"
    );
}

/// An epoch from another attempt cannot authorize this one.
#[tokio::test]
async fn a_commit_quoting_the_wrong_epoch_is_refused() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (_, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");

    let before = calls(&log).len();
    let err = d.commit("some-other-epoch").await.expect_err("wrong epoch");
    assert!(err.contains("is holding a prepare for"), "{err}");
    assert_eq!(calls(&log).len(), before, "no rank may be asked anything");
}

#[tokio::test]
async fn an_abort_releases_every_rank() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (epoch, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");

    d.abort(&epoch).await;
    let c = calls(&log);
    assert!(c.contains(&"local.abort".to_owned()));
    assert!(c.contains(&format!("{}.abort", node_id(2).short())));
    assert!(c.contains(&format!("{}.abort", node_id(3).short())));

    // And the prepare is gone, so it cannot then be committed.
    assert!(d.commit(&epoch).await.is_err());
}

/// An abort for a stale epoch arriving late must not release the prepare made
/// since — that would silently cancel a launch the operator is watching.
#[tokio::test]
async fn a_stale_abort_does_not_release_a_newer_prepare() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let (first, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares");
    let (second, _, _) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("prepares again");
    assert_ne!(first, second, "each attempt gets its own epoch");

    d.abort(&first).await;
    assert!(
        d.commit(&second).await.is_ok(),
        "the newer prepare must survive a stale abort"
    );
}

/// The selection is not a suggestion: a machine the fleet does not know about
/// must fail before anything is asked of anybody.
#[tokio::test]
async fn a_machine_outside_the_fleet_fails_before_any_rank_is_asked() {
    let log = new_log();
    let (d, _) = driver(ready_rank(&log), transport(&log));
    let err = d
        .prepare(
            &recipe(),
            &[node_id(1), node_id(9)],
            node_id(1),
            &BTreeMap::new(),
        )
        .await
        .expect_err("node 9 is not in this fleet");
    assert!(err.contains("not in this fleet"), "{err}");
    assert!(
        calls(&log).is_empty(),
        "nothing may be asked: {:?}",
        calls(&log)
    );
}

/// The head is a rank like any other. A head that skipped its own prepare would
/// commit a rank nobody validated.
#[tokio::test]
async fn the_head_prepares_itself_like_any_other_rank() {
    let log = new_log();
    let (d, _) = driver(refusing_rank(&log, "docker is down"), transport(&log));
    let (_, ranks, may) = d
        .prepare(&recipe(), &all_three(), node_id(1), &BTreeMap::new())
        .await
        .expect("the plan is possible");

    assert!(!may, "the head refusing must block the commit too");
    let head = ranks.iter().find(|r| r.rank == 0).expect("rank 0");
    assert!(!head.prepared);
    assert_eq!(head.reason, "docker is down");
}
