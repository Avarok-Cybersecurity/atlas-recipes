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
/// What [`ClusterControl::preview`] answers: each rank's own rendering, plus a
/// warning when the plan is possible but worth reading twice.
///
/// Named because the method returns it inside a [`crate::BoxFut`], and spelling
/// the whole thing at both the trait and the impl is exactly the repetition
/// `clippy::type_complexity` objects to.
pub type PreviewAnswer = Result<
    (
        Vec<atlasctl_protocol::msg::fleet::RankPreview>,
        Option<String>,
    ),
    String,
>;

/// What [`ClusterControl::prepare`] answers: the epoch a later commit must
/// quote, each rank's reply, and whether a commit may proceed at all.
pub type PrepareAnswer = Result<
    (
        String,
        Vec<atlasctl_protocol::msg::fleet::RankPrepare>,
        bool,
    ),
    String,
>;

/// Every method returns a boxed future rather than being an `async fn`: this
/// trait is used as `dyn ClusterControl`, and an `async fn` in a trait is not
/// dyn-compatible.
pub trait ClusterControl: Send + Sync {
    /// Plan the launch and collect each rank's own rendering. Reserves nothing.
    ///
    /// # Errors
    /// If the plan is impossible, or a rank refuses or cannot be reached.
    fn preview<'a>(
        &'a self,
        recipe: &'a RecipeId,
        nodes: &'a [atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &'a BTreeMap<String, SettingValue>,
    ) -> crate::BoxFut<'a, PreviewAnswer>;

    /// Ask every rank to validate and reserve. Nothing starts.
    ///
    /// Returns the epoch a later commit must quote, each rank's answer, and
    /// whether a commit may proceed. A rank refusing is a normal outcome
    /// reported in the answers, not an error.
    ///
    /// # Errors
    /// If the plan itself is impossible, which is before any rank was asked.
    fn prepare<'a>(
        &'a self,
        recipe: &'a RecipeId,
        nodes: &'a [atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &'a BTreeMap<String, SettingValue>,
    ) -> crate::BoxFut<'a, PrepareAnswer>;

    /// Start what every rank prepared under this epoch.
    ///
    /// # Errors
    /// If no such prepare is outstanding, or a rank fails to start — in which
    /// case every rank that did start has already been stopped.
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
    >;

    /// Abandon a prepare, releasing every reservation.
    fn abort<'a>(&'a self, epoch: &'a str) -> crate::BoxFut<'a, ()>;

    /// Stop every rank of the cluster this agent started.
    ///
    /// # Errors
    /// If no cluster is running, or a rank could not be stopped — and the
    /// error names which, because a rank left running holds a whole GPU.
    fn stop_cluster<'a>(
        &'a self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>,
                > + Send
                + 'a,
        >,
    >;

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
    fn supervise<'a>(&'a self) -> crate::BoxFut<'a, Option<crate::clusterdriver::Torn>>;
}
