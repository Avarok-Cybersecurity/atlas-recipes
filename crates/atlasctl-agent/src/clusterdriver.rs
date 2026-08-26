// SPDX-License-Identifier: AGPL-3.0-only

//! Driving a cluster launch from the head.
//!
//! The head plans — which machines, which ranks, which rendezvous address — and
//! then asks. It does not render another machine's command, because it does not
//! know that machine's recipe revision, flag table or hardware. Preview and
//! execution therefore come from the same code on the same box, which is the
//! only way a preview can be trusted to be what runs.
//!
//! Rank 0 is served locally for exactly the same reason: the head *is* the
//! machine that would run rank 0. It goes through the identical [`RankService`]
//! the peers expose, so there is no shorter, less-checked path for the machine
//! that happens to be holding the plan.
//!
//! ## Why two phases
//!
//! A single-phase launch has no way to fail cleanly. Start ranks one at a time
//! and the third machine's refusal leaves two containers running half a
//! cluster, waiting forever on a rendezvous that will never complete — and the
//! operator sees a hang, not an error. So every rank validates and reserves
//! first, and nothing starts until all of them have said yes.
//!
//! Both phases roll back. A refusal releases every reservation already taken; a
//! failed commit stops every rank already started, including rank 0. The
//! invariant is that a cluster is either whole or absent, never partial.

use anyhow::Result;
use crate::cluster::{PrepareReply, new_epoch};
use crate::fleet::FleetView;
use crate::rank::RankService;
use crate::session::ClusterControl;
use crate::transport::RankTransport;
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::fleet::{DisplayName, NodeId};
use atlasctl_protocol::msg::fleet::{RankPrepare, RankPreview, RankStarted};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One rank, resolved to somewhere reachable.
///
/// Built before any rank is asked anything, so a machine that has left the
/// fleet or has no usable link fails the whole attempt while it is still free
/// to fail — rather than half way through, with reservations already held.
struct Target {
    assignment: crate::cluster::RankAssignment,
    /// Where to reach it, or `None` when it is this machine.
    ///
    /// Recorded rather than re-derived, so commit dials the address prepare
    /// used instead of re-resolving and possibly reaching a different machine.
    addr: Option<SocketAddr>,
    name: DisplayName,
}

/// A cluster that started, kept so it can be stopped again.
///
/// The containers are recorded rather than looked up later: a rank's container
/// is named by that machine, and rediscovering it would mean trusting a name
/// match instead of what the commit actually returned.
struct Running {
    targets: Vec<Target>,
    started: Vec<RankStarted>,
}

/// A prepare that has been accepted and is waiting to be committed.
struct Pending {
    epoch: String,
    port: u16,
    targets: Vec<Target>,
}

/// Plans a cluster and drives it across the fleet.
pub struct ClusterDriver {
    fleet: Arc<dyn FleetView>,
    /// This machine, when it is one of the ranks.
    rank: Arc<dyn RankService>,
    /// Every other machine.
    transport: Arc<dyn RankTransport>,
    peer_port: u16,
    /// How long to let a cluster settle before believing it started.
    ///
    /// Not a readiness wait — weights take minutes to load, and nothing here
    /// waits for that. It is long enough to catch the rank that dies on
    /// startup, which is the failure that otherwise reads as a hang.
    settle: Duration,
    pending: Mutex<Option<Pending>>,
    running: Mutex<Option<Running>>,
}

/// Default settling window.
pub const SETTLE: Duration = Duration::from_secs(5);

impl ClusterDriver {
    /// Shorten the settling window.
    ///
    /// Only for tests: a real cluster needs a window long enough for a doomed
    /// rank to actually die, and zero would make the gate always pass.
    #[must_use]
    pub fn with_settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self
    }
}

impl std::fmt::Debug for ClusterDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterDriver").finish_non_exhaustive()
    }
}

impl ClusterDriver {
    /// Build a driver.
    #[must_use]
    pub fn new(
        fleet: Arc<dyn FleetView>,
        rank: Arc<dyn RankService>,
        transport: Arc<dyn RankTransport>,
        peer_port: u16,
    ) -> Self {
        Self {
            fleet,
            rank,
            transport,
            peer_port,
            settle: SETTLE,
            pending: Mutex::new(None),
            running: Mutex::new(None),
        }
    }

    /// Release every reservation held for this attempt, ignoring failures.
    ///
    /// Failures are ignored on purpose: this runs when something has already
    /// gone wrong, and a second failure must not replace the reason the
    /// operator needs to read. A reservation left behind is released by that
    /// machine's next prepare regardless.
    fn roll_back(&self, epoch: &str, targets: &[&Target]) {
        for t in targets {
            match t.addr {
                None => self.rank.abort(epoch),
                Some(addr) => self.transport.abort(t.assignment.node, addr, epoch),
            }
        }
    }
}

impl ClusterControl for ClusterDriver {
    fn preview(
        &self,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<(Vec<RankPreview>, Option<String>), String> {
        // A preview reserves nothing, so it needs no epoch of its own.
        let pending = self.resolve(recipe, nodes, head, settings, "preview".to_owned())?;

        let mut out = Vec::with_capacity(pending.targets.len());
        for t in &pending.targets {
            let (command, _unmapped) = match t.addr {
                None => self
                    .rank
                    .render(&t.assignment)
                    .map_err(|e| format!("{}: {e:#}", t.name))?,
                Some(addr) => self
                    .transport
                    .preview(t.assignment.node, addr, &t.assignment)
                    .map_err(|e| format!("{}: {e:#}", t.name))?,
            };
            out.push(RankPreview {
                node: t.assignment.node,
                name: t.name.clone(),
                rank: t.assignment.rank,
                master_addr: t.assignment.master_addr.clone(),
                command,
            });
        }
        Ok((out, self.link_warning(recipe, nodes, head, settings)))
    }

    fn prepare(
        &self,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<(String, Vec<RankPrepare>, bool), String> {
        let epoch = new_epoch();
        let pending = self.resolve(recipe, nodes, head, settings, epoch.clone())?;

        let mut answers = Vec::with_capacity(pending.targets.len());
        let mut accepted: Vec<&Target> = Vec::new();
        for t in &pending.targets {
            let reply = match t.addr {
                None => self.rank.prepare(&epoch, &t.assignment),
                Some(addr) => {
                    self.transport
                        .prepare(t.assignment.node, addr, &epoch, &t.assignment)
                }
            };

            let prepared = matches!(reply, PrepareReply::Prepared);
            if prepared {
                accepted.push(t);
            }
            answers.push(RankPrepare {
                node: t.assignment.node,
                name: t.name.clone(),
                rank: t.assignment.rank,
                prepared,
                reason: match reply {
                    PrepareReply::Prepared => String::new(),
                    PrepareReply::Refused { reason } => reason,
                },
            });
        }

        let may_commit = answers.iter().all(|r| r.prepared);
        if may_commit {
            *self.pending.lock().expect("pending lock poisoned") = Some(pending);
        } else {
            // Nothing has started, but reservations are held. Release them now
            // rather than leaving those machines unable to launch until each is
            // prepared again.
            self.roll_back(&epoch, &accepted);
        }
        Ok((epoch, answers, may_commit))
    }

    fn commit(&self, epoch: &str) -> Result<Vec<RankStarted>, String> {
        // Taken, not borrowed: a commit consumes its prepare, so a replayed
        // commit starts nothing a second time.
        let pending = {
            let mut slot = self.pending.lock().expect("pending lock poisoned");
            match slot.as_ref() {
                Some(p) if p.epoch == epoch => slot.take().expect("just matched"),
                Some(p) => {
                    return Err(format!(
                        "this agent is holding a prepare for {}, not {epoch}",
                        p.epoch
                    ));
                }
                None => return Err(format!("no prepare is outstanding for {epoch}")),
            }
        };

        let mut started = Vec::with_capacity(pending.targets.len());
        for (i, t) in pending.targets.iter().enumerate() {
            let result = match t.addr {
                None => self.rank.commit(epoch).map_err(|e| format!("{e:#}")),
                Some(addr) => self
                    .transport
                    .commit(t.assignment.node, addr, epoch)
                    .map_err(|e| format!("{e:#}")),
            };

            match result {
                Ok(container) => started.push(RankStarted {
                    node: t.assignment.node,
                    name: t.name.clone(),
                    rank: t.assignment.rank,
                    container,
                    // Only rank 0 serves the API. A worker's URL would not
                    // answer, and offering one would cost the operator time.
                    endpoint: (t.assignment.rank == 0).then(|| {
                        format!("http://{}:{}", t.assignment.master_addr, pending.port)
                    }),
                }),
                Err(reason) => {
                    // A half-started cluster waits forever on a rendezvous that
                    // will never complete, and the operator sees a hang rather
                    // than an error. Stop what started and release what did
                    // not, then say which machine refused and why.
                    let done: Vec<&Target> = pending.targets[..i].iter().collect();
                    self.stop_started(&started, &done);
                    let rest: Vec<&Target> = pending.targets[i..].iter().collect();
                    self.roll_back(epoch, &rest);
                    return Err(format!("{} could not start: {reason}", t.name));
                }
            }
        }
        // `docker run -d` returning 0 means the container was created, not that
        // the workload survived. Rank 0 once died one second after starting
        // while the commit reported success and rank 1 kept running alone,
        // waiting forever on a rendezvous — the operator saw a hang, not an
        // error. So every rank is asked again after a settling window, and a
        // cluster that is not whole is torn down rather than reported as up.
        if self.settle > Duration::ZERO {
            std::thread::sleep(self.settle);
        }
        let mut dead = Vec::new();
        for t in &pending.targets {
            let Some(r) = started.iter().find(|r| r.node == t.assignment.node) else {
                continue;
            };
            let alive = match t.addr {
                None => self.rank.alive(&r.container).unwrap_or(false),
                // A rank we cannot ask is not a rank we can count.
                Some(addr) => self
                    .transport
                    .alive(t.assignment.node, addr, &r.container)
                    .unwrap_or(false),
            };
            if !alive {
                dead.push(t.name.to_string());
            }
        }
        if !dead.is_empty() {
            self.stop_started(&started, &pending.targets.iter().collect::<Vec<_>>());
            return Err(format!(
                "{} stopped within {}s of starting. \
                 The whole cluster has been shut down; check that machine's container logs.",
                dead.join(" and "),
                self.settle.as_secs().max(1)
            ));
        }

        *self.running.lock().expect("running lock poisoned") = Some(Running {
            targets: pending.targets,
            started: started.clone(),
        });
        Ok(started)
    }

    fn abort(&self, epoch: &str) {
        let taken = {
            let mut slot = self.pending.lock().expect("pending lock poisoned");
            match slot.as_ref() {
                // Only the named prepare: an abort for a stale epoch arriving
                // late must not release a prepare made since.
                Some(p) if p.epoch == epoch => slot.take(),
                _ => None,
            }
        };
        if let Some(p) = taken {
            let all: Vec<&Target> = p.targets.iter().collect();
            self.roll_back(epoch, &all);
        }
    }

    fn stop_cluster(&self) -> Result<Vec<RankStarted>, String> {
        let running = self
            .running
            .lock()
            .expect("running lock poisoned")
            .take()
            .ok_or_else(|| "this agent did not start a cluster".to_owned())?;

        // Every rank is attempted even after one fails: a rank left running
        // holds a whole GPU, so giving up on the first failure would be the
        // most expensive possible response to it.
        let mut failures = Vec::new();
        for r in &running.started {
            let Some(t) = running.targets.iter().find(|t| t.assignment.node == r.node) else {
                continue;
            };
            match t.addr {
                None => {
                    if let Err(e) = self.rank.stop(&r.container) {
                        failures.push(format!("{}: {e:#}", t.name));
                    }
                }
                Some(addr) => self.transport.stop(t.assignment.node, addr, &r.container),
            }
        }
        if failures.is_empty() {
            Ok(running.started)
        } else {
            Err(format!("could not stop {}", failures.join("; ")))
        }
    }
}

impl ClusterDriver {
    /// Stop the ranks that already started, so a failed commit leaves nothing
    /// running. Failures are ignored for the same reason rollback ignores them:
    /// the operator needs the original error, not this one.
    fn stop_started(&self, started: &[RankStarted], targets: &[&Target]) {
        for r in started {
            let Some(t) = targets.iter().find(|t| t.assignment.node == r.node) else {
                continue;
            };
            match t.addr {
                None => {
                    let _ = self.rank.stop(&r.container);
                }
                Some(addr) => self.transport.stop(t.assignment.node, addr, &r.container),
            }
        }
    }
}



#[cfg(test)]
mod cases;
#[cfg(test)]
mod tests;

mod plan;
