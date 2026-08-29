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

use crate::cluster::{PrepareReply, new_epoch};
use crate::fleet::FleetView;
use crate::rank::RankService;
use crate::session::ClusterControl;
use crate::transport::RankTransport;
use anyhow::Result;
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::fleet::{DisplayName, NodeId};
use atlasctl_protocol::msg::fleet::{RankPrepare, RankPreview, RankStarted};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
            let (command, unmapped) = match t.addr {
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
                unmapped,
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
        // Refuse before touching a single machine. Committing a second cluster
        // was destructive both ways: the same recipe had each rank `docker rm -f`
        // the live one mid-service, and a different one left both running while
        // the second commit overwrote the record of the first.
        //
        // Not `RefusalReason::AlreadyRunning`: its text says "on this node",
        // which is wrong for a cluster spanning machines. That variant belongs
        // on the rank path, where it is still unused.
        {
            let held = self.running.lock().expect("running lock poisoned");
            if let Some(r) = held.as_ref() {
                let names: Vec<String> = r.started.iter().map(|s| s.name.to_string()).collect();
                return Err(format!(
                    "a cluster is already running on {}; stop it before starting another",
                    names.join(", ")
                ));
            }
        }

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
                    //
                    // ⚠ KNOWN, and not fixable here. `master_addr` is the
                    // RENDEZVOUS address -- deliberately the fastest link every
                    // worker SHARES, which on a Spark pair is the point-to-point
                    // RoCE fabric (10.10.10.x). That is the right choice for the
                    // ranks talking to each other and the wrong one for the
                    // human: the operator's laptop is not on that fabric, so the
                    // one URL this product hands them times out while the
                    // container serves perfectly.
                    //
                    // Swapping in `preferred_address` does not fix it -- that
                    // also ranks by link class and picks the same fabric. The
                    // address that works is the one the BROWSER reached this
                    // agent on, which is known at the HTTP layer and not here.
                    // Fixing it properly means the endpoint travelling as a port
                    // plus a machine identity and the page composing the URL
                    // from its own connection, which is a protocol and UI change
                    // rather than a line in this function.
                    endpoint: (t.assignment.rank == 0)
                        .then(|| format!("http://{}:{}", t.assignment.master_addr, pending.port)),
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

    fn supervise(&self) -> Option<Torn> {
        // Read the roster under the lock, then release it: asking a peer
        // whether it is alive dials the network, and holding the lock across
        // that would block a stop the operator asked for.
        let (targets, started) = {
            let held = self.running.lock().expect("running lock poisoned");
            let r = held.as_ref()?;
            (
                r.targets
                    .iter()
                    .map(Target::clone_shallow)
                    .collect::<Vec<_>>(),
                r.started.clone(),
            )
        };

        let mut dead = Vec::new();
        for r in &started {
            let Some(t) = targets.iter().find(|t| t.assignment.node == r.node) else {
                continue;
            };
            let alive = match t.addr {
                None => self.rank.alive(&r.container).unwrap_or(false),
                // Unreachable is not alive. A rank we cannot ask is not one we
                // can count as part of a whole cluster — and if the answer is
                // wrong, tearing down is the cheap mistake.
                Some(addr) => self
                    .transport
                    .alive(t.assignment.node, addr, &r.container)
                    .unwrap_or(false),
            };
            if !alive {
                dead.push((t.assignment.node, t.name.to_string()));
            }
        }
        if dead.is_empty() {
            return None;
        }

        let _ = self.stop_cluster();
        // Clear the record even if that stop failed. `stop_cluster` keeps it on
        // failure so an operator can retry, and `prepare` refuses while it
        // exists -- together those would wedge every future launch here, since
        // the rank being torn down is by definition unreachable and its stop
        // always fails. Supervision is not an operator's Stop: it has decided
        // the cluster is over and nobody will retry it. The alert below names
        // the machine still holding a container.
        *self.running.lock().expect("running lock poisoned") = None;
        let names: Vec<String> = dead.iter().map(|(_, n)| n.clone()).collect();
        Some(Torn {
            // The machines are returned, not just their names, so the caller can
            // raise this against the node in the fleet view. Previously this
            // returned prose that went to `eprintln!` on the head's own process
            // and nowhere else: the browser went on showing a running cluster
            // whose endpoint had quietly died, and the Stop button then answered
            // "this agent did not start a cluster".
            nodes: dead.iter().map(|(id, _)| *id).collect(),
            why: format!(
                "{} stopped, so the cluster was torn down. A half cluster waits at a \
                 rendezvous that will never complete while its survivors hold their GPUs.",
                names.join(" and ")
            ),
        })
    }

    fn stop_cluster(&self) -> Result<Vec<RankStarted>, String> {
        // Read, do not TAKE. Removing the record before attempting a stop meant
        // a reported failure could not be retried: Stop again answered "this
        // agent did not start a cluster" while the rank was still holding a GPU.
        let running = self
            .running
            .lock()
            .expect("running lock poisoned")
            .clone()
            .ok_or_else(|| "this agent did not start a cluster".to_owned())?;

        // Every rank is attempted even after one fails: a rank left running
        // holds a whole GPU, so giving up on the first failure would be the
        // most expensive possible response to it.
        let mut failures = Vec::new();
        let mut still_running = Vec::new();
        for r in &running.started {
            let Some(t) = running.targets.iter().find(|t| t.assignment.node == r.node) else {
                continue;
            };
            let outcome = match t.addr {
                None => self.rank.stop(&r.container).map_err(|e| format!("{e:#}")),
                // Remote failures count now. This arm returned `()`, so an
                // unreachable peer contributed nothing and the operator was told
                // every rank had stopped.
                Some(addr) => self
                    .transport
                    .stop(t.assignment.node, addr, &r.container)
                    .map_err(|e| format!("{e:#}")),
            };
            if let Err(e) = outcome {
                failures.push(format!("{}: {e}", t.name));
                still_running.push(r.clone());
            }
        }

        let mut held = self.running.lock().expect("running lock poisoned");
        if failures.is_empty() {
            *held = None;
            Ok(running.started)
        } else {
            // Keep exactly what is still up, so a retry targets that and not the
            // ranks already stopped -- which would fail on a container that is
            // gone and look like a new problem.
            *held = Some(Running {
                started: still_running,
                ..running.clone()
            });
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
                // Best effort: supervision already knows something is wrong
                // and has no operator waiting on a return value.
                Some(addr) => {
                    let _ = self.transport.stop(t.assignment.node, addr, &r.container);
                }
            }
        }
    }
}

mod types;
pub(crate) use types::*;

#[cfg(test)]
mod cases;
#[cfg(test)]
mod teardown;
#[cfg(test)]
mod tests;

mod plan;
