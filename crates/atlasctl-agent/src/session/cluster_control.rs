// SPDX-License-Identifier: AGPL-3.0-only

//! What the session asks of a cluster.
//!
//! Split from `session.rs` on the 500-line cap, along the trait boundary:
//! this file is the contract the cluster driver implements, its parent the
//! state machine that drives it. Exact piecewise copy; nothing changed in the
//! move.

use atlasctl_protocol::RecipeId;
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;

/// Asks every rank of a planned cluster what it would run.
///
/// A trait so the session stays transport-free: the preview needs to reach
/// other machines, and a session that opened sockets itself could not be tested
/// without them.
pub trait ClusterControl: Send + Sync {
    /// Plan the launch and collect each rank's own rendering. Reserves nothing.
    ///
    /// # Errors
    /// If the plan is impossible, or a rank refuses or cannot be reached.
    fn preview(
        &self,
        recipe: &RecipeId,
        nodes: &[atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<
        (
            Vec<atlasctl_protocol::msg::fleet::RankPreview>,
            Option<String>,
        ),
        String,
    >;

    /// Ask every rank to validate and reserve. Nothing starts.
    ///
    /// Returns the epoch a later commit must quote, each rank's answer, and
    /// whether a commit may proceed. A rank refusing is a normal outcome
    /// reported in the answers, not an error.
    ///
    /// # Errors
    /// If the plan itself is impossible, which is before any rank was asked.
    fn prepare(
        &self,
        recipe: &RecipeId,
        nodes: &[atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<
        (
            String,
            Vec<atlasctl_protocol::msg::fleet::RankPrepare>,
            bool,
        ),
        String,
    >;

    /// Start what every rank prepared under this epoch.
    ///
    /// # Errors
    /// If no such prepare is outstanding, or a rank fails to start — in which
    /// case every rank that did start has already been stopped.
    fn commit(
        &self,
        epoch: &str,
    ) -> Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>;

    /// Abandon a prepare, releasing every reservation.
    fn abort(&self, epoch: &str);

    /// Stop every rank of the cluster this agent started.
    ///
    /// # Errors
    /// If no cluster is running, or a rank could not be stopped — and the
    /// error names which, because a rank left running holds a whole GPU.
    fn stop_cluster(&self) -> Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>;

    /// Check a running cluster is still whole, and tear it down if it is not.
    ///
    /// The settle gate at commit is a liveness check by construction: weights
    /// take minutes to load, so it cannot wait for readiness and only catches
    /// a rank that dies immediately. A rank that dies four minutes in — during
    /// model build, say — passed that gate, and the survivors then hold their
    /// GPUs indefinitely serving nothing, because a half cluster waits at a
    /// rendezvous that will never complete.
    ///
    /// Returns a description of what it tore down, or `None` when the cluster
    /// is whole or absent.
    fn supervise(&self) -> Option<crate::clusterdriver::Torn>;
}
