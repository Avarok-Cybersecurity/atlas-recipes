// SPDX-License-Identifier: AGPL-3.0-only

//! The grant matrix, version skew and budget halves of the three-agent
//! suite (harness in `relay_harness.rs`, adversarial routing cases in
//! `relay_tests.rs`).

use super::relay_harness::*;
use crate::launcher::Launcher;
use crate::session::ControlRelay as _;
use atlasctl_protocol::msg::{AgentError, ControlRep, ControlReq};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// The confused-deputy matrix, in one narrative because the cases share a
/// fleet: no grant at the relay, grant at the relay but not the target,
/// both, and a revocation between two requests with nothing restarted.
#[tokio::test(flavor = "multi_thread")]
async fn the_grant_is_checked_independently_at_relay_and_target() {
    let origin = agent("gm-origin", "macbook", "127.0.0.1");
    let mut relay = agent("gm-relay", "dgx1", "127.0.0.2");
    let mut target = agent("gm-target", "dgx2", "127.0.0.3");
    pin(&origin, &relay, false);
    // Case one starts with NO grant anywhere.
    pin(&relay, &origin, false);
    pin(&relay, &target, false);
    pin(&target, &relay, false);

    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, Duration::from_secs(5), launcher)
    }
    .await;
    {
        let launcher = std::sync::Arc::clone(&target.launcher);
        spawn_serving(&mut target, port, Duration::from_secs(5), launcher)
    }
    .await;
    poll(&relay, &target).await;
    poll(&origin, &relay).await;
    let d = driver(&origin, port);

    // (1) No grant at the relay: refused there, target never dialled.
    let dials_before = target.accepted.load(Ordering::SeqCst);
    let (rep, via) = d
        .control(target.id(), status())
        .await
        .expect("relay answers");
    assert_eq!(via, Some(relay.id()));
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { detail, .. },
        } => {
            assert_eq!(by, relay.id());
            assert!(
                detail.contains("peer grant-control"),
                "the refusal names the exact fix: {detail}"
            );
        }
        other => panic!("expected the relay's R2 refusal, got {other:?}"),
    }
    assert_eq!(
        target.accepted.load(Ordering::SeqCst),
        dials_before,
        "an ungranted requester must not cause the target to be dialled"
    );

    // (2) Grant at the relay, none at the target: the TARGET refuses.
    // R2 alone is insufficient — this is what proves T2 exists.
    assert!(relay.pins.set_controller(origin.id(), true).expect("grant"));
    let (rep, via) = d
        .control(target.id(), status())
        .await
        .expect("chain answers");
    assert_eq!(via, Some(relay.id()));
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::ControlRefused { reason, .. },
        } => {
            assert_eq!(by, target.id(), "the TARGET's own refusal, by name");
            assert!(reason.contains("peer grant-control"), "got {reason}");
        }
        other => panic!("expected the target's T2 refusal, got {other:?}"),
    }

    // (3) Both grants: the verb executes on the target.
    assert!(target.pins.set_controller(relay.id(), true).expect("grant"));
    let (rep, via) = d
        .control(target.id(), status())
        .await
        .expect("chain answers");
    assert!(matches!(rep, ControlRep::Status { .. }), "got {rep:?}");
    assert_eq!(via, Some(relay.id()));

    // (4) Revoked between two requests, nothing restarted: the per-frame
    // pin re-read makes the revocation immediate.
    assert!(
        relay
            .pins
            .set_controller(origin.id(), false)
            .expect("revoke")
    );
    let (rep, _) = d
        .control(target.id(), status())
        .await
        .expect("relay answers");
    assert!(
        matches!(
            rep,
            ControlRep::Refused {
                error: AgentError::RelayRefused { .. },
                ..
            }
        ),
        "a revocation that needs a restart is not a revocation: {rep:?}"
    );
}

/// The direct leg of the same matrix, over real TLS: pinned is not
/// authorized, and a direct answer states `via: None`.
#[tokio::test(flavor = "multi_thread")]
async fn a_directly_pinned_target_still_requires_its_own_grant() {
    let origin = agent("dg-origin", "macbook", "127.0.0.1");
    let mut target = agent("dg-target", "dgx2", "127.0.0.3");
    pin(&origin, &target, false);
    pin(&target, &origin, false);
    let port = {
        let launcher = std::sync::Arc::clone(&target.launcher);
        spawn_serving(&mut target, 0, Duration::from_secs(5), launcher)
    }
    .await;
    poll(&origin, &target).await;
    let d = driver(&origin, port);

    let (rep, via) = d
        .control(target.id(), status())
        .await
        .expect("target answers");
    assert_eq!(via, None, "provenance: a direct dial names no relay");
    assert!(
        matches!(
            rep,
            ControlRep::Refused {
                error: AgentError::ControlRefused { .. },
                ..
            }
        ),
        "pinned must not mean authorized: {rep:?}"
    );

    assert!(
        target
            .pins
            .set_controller(origin.id(), true)
            .expect("grant")
    );
    let (rep, via) = d
        .control(target.id(), status())
        .await
        .expect("target answers");
    assert!(matches!(rep, ControlRep::Status { .. }), "got {rep:?}");
    assert_eq!(via, None);
}

/// O5 at the wire: a peer whose hello advertises no `version_max` is a v1
/// build; the refusal is local, typed, names the version, and no control
/// frame is ever written at the old build.
#[tokio::test(flavor = "multi_thread")]
async fn a_v1_peer_is_refused_by_version_and_receives_no_control_frame() {
    let origin = agent("v1-origin", "macbook", "127.0.0.1");
    let old = agent("v1-old", "dgx-old", "127.0.0.2");
    pin(&origin, &old, false);
    pin(&old, &origin, true);
    let (port, frames) = spawn_fake_peer(&old, 0, None, Duration::ZERO).await;

    let err = driver(&origin, port)
        .control(old.id(), status())
        .await
        .expect_err("must refuse locally");
    match err {
        AgentError::NotRoutable { node, reason } => {
            assert_eq!(node, old.id());
            assert!(reason.contains("peer protocol 1"), "got {reason}");
        }
        other => panic!("expected a typed version refusal, got {other:?}"),
    }
    assert!(
        frames.lock().expect("lock").is_empty(),
        "no v2 frame may ever be written at a v1 build"
    );
}

/// Budgets: a target that outlives the relay's answer budget produces the
/// RELAY's timeout refusal, which reaches the origin well inside its own
/// budget — the strict ordering that prevents the mutual-deadlock shape.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_target_times_out_at_the_relay_not_at_the_origin() {
    let origin = agent("slow-origin", "macbook", "127.0.0.1");
    let mut relay = agent("slow-relay", "dgx1", "127.0.0.2");
    let target = agent("slow-target", "dgx2", "127.0.0.3");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    pin(&relay, &target, false);
    pin(&target, &relay, false);

    // A test-scaled relay budget; the production value would make this a
    // one-minute test without changing what it proves.
    let relay_budget = Duration::from_millis(300);
    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, relay_budget, launcher)
    }
    .await;
    // The fake target answers a Control only after twice the relay budget.
    let (_, _frames) = spawn_fake_peer(&target, port, Some(2), relay_budget * 2).await;
    poll(&origin, &relay).await;
    origin
        .fleet
        .record_vouches(relay.id(), vec![claim(target.id(), Vec::new(), true)]);
    relay
        .fleet
        .record_report(fake_report_of(&target, "127.0.0.3"));

    let started = std::time::Instant::now();
    let (rep, via) = driver(&origin, port)
        .control(target.id(), status())
        .await
        .expect("the relay answers with its own timeout");
    let waited = started.elapsed();
    assert!(
        waited < crate::peer::control::ORIGIN_ANSWER_BUDGET,
        "the origin must still have budget left when the relay gives up"
    );
    assert_eq!(via, Some(relay.id()));
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::RelayRefused { detail, .. },
        } => {
            assert_eq!(by, relay.id());
            assert!(detail.contains("did not answer"), "got {detail}");
        }
        other => panic!("expected the relay's timeout refusal, got {other:?}"),
    }
}

/// A launcher whose first launch outlives the relay budget but SUCCEEDS, and
/// whose second is already running — the lost-reply recovery story.
struct SlowThenBusy {
    launches: AtomicUsize,
    delay: Duration,
}

impl Launcher for SlowThenBusy {
    fn preview(
        &self,
        _: &atlasctl_core::Recipe,
        _: &BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Preview, AgentError> {
        Err(AgentError::NotReady)
    }
    fn launch(
        &self,
        _: &atlasctl_core::Recipe,
        _: &BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Started, AgentError> {
        if self.launches.fetch_add(1, Ordering::SeqCst) > 0 {
            return Err(AgentError::AlreadyRunning { recipe: recipe() });
        }
        std::thread::sleep(self.delay);
        Ok(crate::launcher::Started {
            container: "atlas-slow".to_owned(),
            endpoint: None,
        })
    }
    fn stop(&self, _: &str) -> Result<(), AgentError> {
        Ok(())
    }
    fn running(&self) -> Result<Vec<atlasctl_protocol::msg::RunningLaunch>, AgentError> {
        Ok(Vec::new())
    }
}

/// The no-idempotency-token design, proven: a launch whose reply was lost to
/// the relay's timeout has still HAPPENED, and the retry answers
/// `AlreadyRunning` from the launcher's own state instead of starting a
/// second copy.
///
/// `worker_threads` is pinned rather than left to the machine. `SlowThenBusy`
/// blocks its worker with `thread::sleep`, and `multi_thread` sizes the pool
/// from the CPU count — so on a two-core runner the blocked launch can starve
/// the relay's own timeout timer, which then never fires and the relay returns
/// `Started` for a launch it was supposed to give up on. Observed exactly that
/// on Windows CI. Four workers is enough that a single blocking launch cannot
/// take the runtime with it, and the test still measures what it says.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retried_launch_after_a_lost_reply_is_already_running_not_doubled() {
    let origin = agent("retry-origin", "macbook", "127.0.0.1");
    let mut relay = agent("retry-relay", "dgx1", "127.0.0.2");
    let mut target = agent("retry-target", "dgx2", "127.0.0.3");
    pin(&origin, &relay, false);
    pin(&relay, &origin, true);
    pin(&relay, &target, false);
    pin(&target, &relay, true);

    // The margin between these two is the whole test: the relay must give up
    // BEFORE the launch returns, or the reply is not lost and there is nothing
    // to recover from. It used to be 3x (300ms vs 900ms), and a loaded Windows
    // runner lost that race -- the launch answered `Started`, the assertion for
    // the relay's refusal failed, and a pull request that had not touched any
    // Rust went red. 20x is not a tuned number; it is far enough outside any
    // plausible scheduling delay that the ordering stops being a coin flip.
    let relay_budget = Duration::from_millis(150);
    let launcher = Arc::new(SlowThenBusy {
        launches: AtomicUsize::new(0),
        delay: relay_budget * 20,
    });
    let port = {
        let launcher = std::sync::Arc::clone(&relay.launcher);
        spawn_serving(&mut relay, 0, relay_budget, launcher)
    }
    .await;
    spawn_serving(
        &mut target,
        port,
        Duration::from_secs(5),
        Arc::clone(&launcher) as Arc<dyn crate::launcher::Launcher>,
    )
    .await;
    poll(&relay, &target).await;
    poll(&origin, &relay).await;
    let d = driver(&origin, port);

    let launch = ControlReq::Launch {
        recipe: recipe(),
        settings: BTreeMap::new(),
    };
    let (rep, _) = d
        .control(target.id(), launch.clone())
        .await
        .expect("answers");
    assert!(
        matches!(
            rep,
            ControlRep::Refused {
                error: AgentError::RelayRefused { .. },
                ..
            }
        ),
        "the first attempt's reply is lost to the relay's budget: {rep:?}"
    );

    // Let the slow launch actually finish before retrying.
    tokio::time::sleep(relay_budget * 4).await;
    let (rep, _) = d.control(target.id(), launch).await.expect("answers");
    match rep {
        ControlRep::Refused {
            by,
            error: AgentError::AlreadyRunning { .. },
        } => assert_eq!(by, target.id(), "the launcher's state is the SSOT"),
        other => panic!("expected AlreadyRunning, got {other:?}"),
    }
    assert_eq!(
        launcher.launches.load(Ordering::SeqCst),
        2,
        "two attempts, ONE actual launch: the second was refused by state"
    );
}

/// A live report for a fake peer, the way the relay's poll loop would hold
/// one — fake peers answer no digest-bearing poll of their own.
fn fake_report_of(a: &TestAgent, ip: &str) -> crate::peer::link::PeerReport {
    crate::peer::link::PeerReport {
        node: a.id(),
        name: "fake".to_owned(),
        can_launch: true,
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        link: atlasctl_protocol::fleet::LinkClass::Ethernet,
        addresses: vec![atlasctl_protocol::fleet::NodeAddress {
            iface: "lo".to_owned(),
            addr: ip.to_owned(),
            class: atlasctl_protocol::fleet::LinkClass::Ethernet,
            speed_mbps: None,
            prefix_len: 8,
            rdma: false,
        }],
        vouched: None,
        peer_version_max: 2,
    }
}

/// A refusal is read on a laptop, about a machine that is not the laptop.
///
/// The grant has to be made on the machine that refused, and every hop of this
/// design is a different box — so the remedy has to name which one. "Run it on
/// this machine" sends the operator to whichever machine they are looking at,
/// where the command succeeds, changes nothing relevant, and leaves them
/// refused again with no new information.
#[test]
fn a_grant_refusal_names_the_machine_to_run_the_command_on() {
    let sender = atlasctl_protocol::fleet::NodeId::from_bytes([3; 32]);
    let local = atlasctl_protocol::fleet::NodeId::from_bytes([9; 32]);
    let msg = crate::daemon::peer_serve::grant_refusal(sender, local);

    assert!(
        msg.contains("grant-control"),
        "it must name the command: {msg}"
    );
    assert!(
        msg.contains(&local.short()),
        "it must name the machine to run it ON: {msg}"
    );
    assert!(
        msg.contains(&sender.short()),
        "it must name the machine being granted: {msg}"
    );
    assert!(
        !msg.contains("this machine"),
        "\"this machine\" is whichever box the operator is looking at, which is \
         never the one that refused: {msg}"
    );
}
