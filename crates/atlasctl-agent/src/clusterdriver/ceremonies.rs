// SPDX-License-Identifier: AGPL-3.0-only

//! The cluster ceremonies: preview, prepare, commit, abort, stop, supervise.
//!
//! Split from `clusterdriver.rs` for the 500-line cap. The seam is the one that
//! already existed: [`super::ClusterControl`] is the boxed-future surface, and
//! these are the ordinary async functions behind it.

use super::*;

/// The cluster ceremonies, as ordinary async functions.
///
/// [`ClusterControl`] exposes them as boxed futures because it is used as a
/// trait object; keeping the real bodies here means the boxing is one line each
/// and nothing about the ceremony is shaped by that constraint.
impl ClusterDriver {
    pub(super) async fn preview_inner(
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
                    .await
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

    pub(super) async fn prepare_inner(
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
                None => {
                    // The local rank shells out to docker exactly as a remote
                    // one does; the only difference is the absence of a network
                    // hop. Keeping it inline would hold a worker for the whole
                    // reservation while the remote ranks' calls do not.
                    let rank = std::sync::Arc::clone(&self.rank);
                    let epoch = epoch.clone();
                    let assignment = t.assignment.clone();
                    local_blocking(move || rank.prepare(&epoch, &assignment)).await
                }
                Some(addr) => {
                    self.transport
                        .prepare(t.assignment.node, addr, &epoch, &t.assignment)
                        .await
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
            self.roll_back(&epoch, &accepted).await;
        }
        Ok((epoch, answers, may_commit))
    }

    pub(super) async fn commit_inner(&self, epoch: &str) -> Result<Vec<RankStarted>, String> {
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
                None => {
                    let rank = std::sync::Arc::clone(&self.rank);
                    let epoch = epoch.to_owned();
                    local_blocking(move || rank.commit(&epoch))
                        .await
                        .map_err(|e| format!("{e:#}"))
                }
                Some(addr) => self
                    .transport
                    .commit(t.assignment.node, addr, epoch)
                    .await
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
                    self.stop_started(&started, &done).await;
                    let rest: Vec<&Target> = pending.targets[i..].iter().collect();
                    self.roll_back(epoch, &rest).await;
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
                None => {
                    let rank = std::sync::Arc::clone(&self.rank);
                    let container = r.container.clone();
                    local_blocking(move || rank.alive(&container))
                        .await
                        .unwrap_or(false)
                }
                // A rank we cannot ask is not a rank we can count.
                Some(addr) => self
                    .transport
                    .alive(t.assignment.node, addr, &r.container)
                    .await
                    .unwrap_or(false),
            };
            if !alive {
                dead.push(t.name.to_string());
            }
        }
        if !dead.is_empty() {
            self.stop_started(&started, &pending.targets.iter().collect::<Vec<_>>())
                .await;
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

    pub(super) async fn abort_inner(&self, epoch: &str) {
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
            self.roll_back(epoch, &all).await;
        }
    }

    pub(super) async fn stop_cluster_inner(&self) -> Result<Vec<RankStarted>, String> {
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
                None => {
                    let rank = std::sync::Arc::clone(&self.rank);
                    let container = r.container.clone();
                    local_blocking(move || rank.stop(&container))
                        .await
                        .map_err(|e| format!("{e:#}"))
                }
                // Remote failures count now. This arm returned `()`, so an
                // unreachable peer contributed nothing and the operator was told
                // every rank had stopped.
                Some(addr) => self
                    .transport
                    .stop(t.assignment.node, addr, &r.container)
                    .await
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

    pub(super) async fn supervise_inner(&self) -> Option<Torn> {
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
                None => {
                    let rank = std::sync::Arc::clone(&self.rank);
                    let container = r.container.clone();
                    local_blocking(move || rank.alive(&container))
                        .await
                        .unwrap_or(false)
                }
                // Unreachable is not alive. A rank we cannot ask is not one we
                // can count as part of a whole cluster — and if the answer is
                // wrong, tearing down is the cheap mistake.
                Some(addr) => self
                    .transport
                    .alive(t.assignment.node, addr, &r.container)
                    .await
                    .unwrap_or(false),
            };
            if !alive {
                dead.push((t.assignment.node, t.name.to_string()));
            }
        }
        if dead.is_empty() {
            return None;
        }

        let _ = self.stop_cluster_inner().await;
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
}

/// Run a blocking local-rank call on tokio's blocking pool.
///
/// `RankService`'s prepare/commit/alive/stop shell out to docker. The trait
/// stays synchronous — it has two implementations and a large test surface, and
/// nothing about it needs to know where it runs — so the move happens here, at
/// the call site, which is possible because `rank` is already an `Arc` and the
/// arguments are owned.
///
/// `render`, `content_hash` and `recipe_port` are NOT routed through this: they
/// are registry lookups with no I/O, and a hop to the blocking pool would cost
/// more than the work.
pub(super) async fn local_blocking<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(v) => v,
        // A panic in the local rank is a bug, and there is no error channel
        // shared by all four call sites; re-raising keeps it visible rather
        // than turning it into a plausible-looking "not alive".
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}
