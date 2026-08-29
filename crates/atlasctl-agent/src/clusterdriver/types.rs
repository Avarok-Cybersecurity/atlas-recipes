// SPDX-License-Identifier: AGPL-3.0-only

//! The driver's own record types, split from [`super`] for size.
//!
//! Types rather than methods, because a trait impl cannot span files -- the
//! obvious seam (supervision + stop) is inside `impl ClusterControl` and had to
//! stay put.

use super::*;

///
/// Built before any rank is asked anything, so a machine that has left the
/// fleet or has no usable link fails the whole attempt while it is still free
/// to fail — rather than half way through, with reservations already held.
#[derive(Clone)]
/// One rank, resolved to somewhere reachable.
/// A prepare that has been accepted and is waiting to be committed.
pub(crate) struct Target {
    pub(crate) assignment: crate::cluster::RankAssignment,
    /// Where to reach it, or `None` when it is this machine.
    ///
    /// Recorded rather than re-derived, so commit dials the address prepare
    /// used instead of re-resolving and possibly reaching a different machine.
    pub(crate) addr: Option<SocketAddr>,
    pub(crate) name: DisplayName,
}

/// What supervision found, when it found something.
///
/// Carries the machines as well as the sentence, so the caller can raise it
/// where an operator will see it rather than only writing it to the head's
/// stderr.
pub struct Torn {
    /// The machines that stopped answering.
    pub nodes: Vec<NodeId>,
    /// One sentence, already safe to render.
    pub why: String,
}

/// A cluster that started, kept so it can be stopped again.
///
/// The containers are recorded rather than looked up later: a rank's container
/// is named by that machine, and rediscovering it would mean trusting a name
/// match instead of what the commit actually returned.
#[derive(Clone)]
pub(crate) struct Running {
    pub(crate) targets: Vec<Target>,
    pub(crate) started: Vec<RankStarted>,
}

impl Target {
    /// Enough of a target to ask whether its rank is alive.
    pub(crate) fn clone_shallow(&self) -> Self {
        Self {
            assignment: self.assignment.clone(),
            addr: self.addr,
            name: self.name.clone(),
        }
    }
}

pub(crate) struct Pending {
    pub(crate) epoch: String,
    pub(crate) port: u16,
    pub(crate) targets: Vec<Target>,
}

#[cfg(test)]
impl ClusterDriver {
    /// Mark a rank's container dead, so supervision has something to find.
    pub(crate) fn kill_for_test(&self, node: NodeId) {
        self.transport.kill_for_test(node);
    }
}

impl ClusterDriver {
    /// Stop the ranks that already started, so a failed commit leaves nothing
    /// running. Failures are ignored for the same reason rollback ignores them:
    /// the operator needs the original error, not this one.
    pub(crate) fn stop_started(&self, started: &[RankStarted], targets: &[&Target]) {
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
