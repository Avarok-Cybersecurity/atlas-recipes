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
    async fn roll_back(&self, epoch: &str, targets: &[&Target]) {
        for t in targets {
            match t.addr {
                None => self.rank.abort(epoch),
                Some(addr) => self.transport.abort(t.assignment.node, addr, epoch).await,
            }
        }
    }
}

impl ClusterControl for ClusterDriver {
    fn preview<'a>(
        &'a self,
        recipe: &'a RecipeId,
        nodes: &'a [atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &'a BTreeMap<String, SettingValue>,
    ) -> crate::BoxFut<'a, crate::session::PreviewAnswer> {
        Box::pin(self.preview_inner(recipe, nodes, head, settings))
    }

    fn prepare<'a>(
        &'a self,
        recipe: &'a RecipeId,
        nodes: &'a [atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &'a BTreeMap<String, SettingValue>,
    ) -> crate::BoxFut<'a, crate::session::PrepareAnswer> {
        Box::pin(self.prepare_inner(recipe, nodes, head, settings))
    }

    fn commit<'a>(
        &'a self,
        epoch: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(self.commit_inner(epoch))
    }

    fn abort<'a>(&'a self, epoch: &'a str) -> crate::BoxFut<'a, ()> {
        Box::pin(self.abort_inner(epoch))
    }

    fn stop_cluster<'a>(
        &'a self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(self.stop_cluster_inner())
    }

    fn supervise<'a>(&'a self) -> crate::BoxFut<'a, Option<crate::clusterdriver::Torn>> {
        Box::pin(self.supervise_inner())
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

mod ceremonies;
