// SPDX-License-Identifier: AGPL-3.0-only

//! The message catalogue.
//!
//! Internally tagged on `type`, which maps directly onto a TypeScript
//! discriminated union so the browser narrows on `msg.type` without unwrapping
//! a nesting level.
//!
//! The set of client messages *is* the capability surface. There is no relay,
//! no forward, and no raw-command verb, and the enum is closed — an unknown
//! `type` fails deserialization rather than reaching a handler. That is what
//! keeps the agent from becoming an open proxy for whatever page is talking
//! to it.

use crate::id::RecipeId;
use crate::msg::fleet::{FleetEvent, RankPrepare, RankPreview, RankStarted};
use crate::settings::{SettingError, SettingSpec, SettingValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a client may ask for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First frame. Anything else before this is refused.
    Hello {
        /// Protocol version the client chose.
        protocol_version: u32,
        /// Pairing token, proving the user connected this browser deliberately.
        token: String,
    },

    /// Ask for the recipe inventory again.
    ListRecipes {
        /// Correlates the reply.
        id: u32,
    },

    /// Render the command a launch would run, without running it.
    Preview {
        /// Correlates the reply.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },

    /// Start a recipe.
    Launch {
        /// Correlates the reply.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// Requested settings, validated against the schema before use.
        #[serde(default)]
        settings: BTreeMap<String, SettingValue>,
    },

    /// Stop a running launch.
    Stop {
        /// Correlates the reply.
        id: u32,
        /// Which recipe to stop.
        recipe: RecipeId,
    },

    /// Ask for the current fleet: this node and every peer it knows about.
    ListNodes {
        /// Correlates the reply.
        id: u32,
    },

    /// Subscribe to fleet changes — node arrivals, vitals, alerts, run phases.
    ///
    /// Subscription rather than polling so the page can be quiet when nothing
    /// is happening, which is most of the time.
    WatchFleet {
        /// Correlates the reply.
        id: u32,
        /// Whether to receive vitals samples, or only structural changes. A
        /// background tab keeps structure and drops the 1 Hz telemetry.
        vitals: bool,
    },

    /// Begin pairing with a discovered peer, using a code read off that machine.
    ///
    /// The code is never generated here. It originates on the target, which is
    /// what stops a web page from pairing anything on its own.
    PairPeer {
        /// Correlates the reply.
        id: u32,
        /// Which discovered peer.
        node: crate::fleet::NodeId,
        /// The digits the user read off the other machine.
        code: String,
    },

    /// Open a window in which one new machine may join this fleet.
    ///
    /// The inverse direction of [`Self::PairPeer`]: this machine mints the code
    /// and the operator carries it to the machine being added. It exists
    /// because the other direction needs a screen on the target, which a
    /// headless box does not have.
    ///
    /// **This does not weaken the rule that a web page cannot pair on its own.**
    /// The code still has to physically reach another machine, and only a
    /// person can do that. What it does mean is that the code passes through a
    /// shell, so the window is short, single-use, and attempt-limited.
    MintJoinCode {
        /// Correlates the reply.
        id: u32,
    },

    /// Close an outstanding join window without using it.
    RevokeJoinCode {
        /// Correlates the reply.
        id: u32,
    },

    /// Drop trust in a peer. Takes effect on its next connection attempt.
    UnpairPeer {
        /// Correlates the reply.
        id: u32,
        /// Which peer.
        node: crate::fleet::NodeId,
    },

    /// Render the per-rank commands a cluster launch would run, without running
    /// them. Each rank's command is rendered by that rank's own agent, so the
    /// preview cannot drift from what executes.
    PreviewCluster {
        /// Correlates the reply.
        id: u32,
        /// Recipe to launch.
        recipe: RecipeId,
        /// Nodes to use, in no particular order.
        nodes: Vec<crate::fleet::NodeId>,
        /// Which node serves rank 0.
        head: crate::fleet::NodeId,
        /// Bounded overrides.
        settings: BTreeMap<String, SettingValue>,
    },

    /// Ask every selected node to validate and reserve. Nothing starts.
    PrepareCluster {
        /// Correlates the reply.
        id: u32,
        /// Recipe to launch.
        recipe: RecipeId,
        /// Nodes to use.
        nodes: Vec<crate::fleet::NodeId>,
        /// Which node serves rank 0.
        head: crate::fleet::NodeId,
        /// Bounded overrides.
        settings: BTreeMap<String, SettingValue>,
    },

    /// Start every rank of a prepared cluster.
    ///
    /// The epoch pins this to one prepare, so a stale prepare cannot be
    /// committed against a plan that has since changed.
    CommitCluster {
        /// Correlates the reply.
        id: u32,
        /// Epoch from the prepare reply.
        epoch: String,
    },

    /// Abandon a prepare, releasing every reservation.
    AbortCluster {
        /// Correlates the reply.
        id: u32,
        /// Epoch from the prepare reply.
        epoch: String,
    },

    /// Stop every rank of the cluster this agent started.
    ///
    /// Named without a recipe or a node list on purpose: an agent stops the
    /// cluster it launched and knows the containers for, rather than accepting
    /// a list of things to stop on machines it was told about. A page cannot
    /// use its local agent to stop something it did not start.
    StopCluster {
        /// Correlates the reply.
        id: u32,
    },

    /// Ask how a running launch is doing.
    ///
    /// Polled rather than streamed. A sample is a difference between two
    /// scrapes, so the page asking sets the window it gets — and a page that
    /// stops asking costs the agent nothing, which a stream would not.
    LaunchStats {
        /// Correlates the reply.
        id: u32,
        /// Which launch.
        recipe: RecipeId,
    },

    /// Read the tail of a launch's log.
    ///
    /// A tail, not a stream. Weight loading takes minutes and produces a lot of
    /// output; a page that asks for the last N lines when it wants them costs
    /// the agent nothing between asks, and cannot fall behind a firehose it
    /// then has to be told it missed.
    LaunchLogs {
        /// Correlates the reply.
        id: u32,
        /// Which launch.
        recipe: RecipeId,
        /// How many lines to return, capped by the agent.
        lines: u32,
    },

    /// Ask what is running.
    Status {
        /// Correlates the reply.
        id: u32,
    },
}

/// What the agent says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Sent unsolicited as soon as the connection opens, before authentication,
    /// so a client can report a version mismatch rather than timing out.
    Welcome {
        /// Lowest protocol version this agent speaks.
        protocol_min: u32,
        /// Highest protocol version this agent speaks.
        protocol_max: u32,
        /// Agent version, for display.
        agent_version: String,
    },

    /// Authentication succeeded; here is everything the client needs.
    Ready {
        /// Negotiated protocol version.
        protocol_version: u32,
        /// The full settings schema, so the client renders what we validate.
        schema: Vec<SettingSpec>,
        /// Every recipe this agent can launch.
        recipes: Vec<RecipeInfo>,
        /// Whether this machine can actually run a recipe.
        can_launch: bool,
        /// Why not, when it cannot.
        can_launch_reason: Option<String>,
    },

    /// Reply to `ListRecipes`.
    Recipes {
        /// Correlation id.
        id: u32,
        /// The inventory.
        recipes: Vec<RecipeInfo>,
    },

    /// Reply to `Preview`.
    Preview {
        /// Correlation id.
        id: u32,
        /// The exact command, shell-quoted for display.
        command: String,
        /// Settings the recipe carries that this agent does not understand.
        ///
        /// Surfaced rather than dropped: the tool this replaces discarded these
        /// silently, which is how a stated correctness pin went unapplied for
        /// months without anyone noticing.
        unapplied: Vec<String>,
    },

    /// A launch was accepted and started.
    Started {
        /// Correlation id.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// The container name, for logs and stop.
        container: String,
        /// Where the model is served, when it serves.
        endpoint: Option<String>,
    },

    /// Reply to `Status`.
    Status {
        /// Correlation id.
        id: u32,
        /// What is running.
        running: Vec<RunningLaunch>,
    },

    /// A launch was stopped.
    Stopped {
        /// Correlation id.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
    },

    /// The fleet, in full. Sent in reply to `ListNodes` and once on `WatchFleet`.
    Nodes {
        /// Correlates the request.
        id: u32,
        /// This node first, then peers.
        nodes: Vec<crate::fleet::NodeDescriptor>,
    },

    /// One thing changed about the fleet.
    ///
    /// Sent unsolicited to a watcher. Vitals are coalesced newest-wins by the
    /// agent — losing a 1 Hz sample costs nothing — while structural changes
    /// and alerts are never dropped.
    FleetEvent {
        /// What happened.
        event: FleetEvent,
    },

    /// A pairing finished, one way or the other.
    PairResult {
        /// Correlates the request.
        id: u32,
        /// The peer.
        node: crate::fleet::NodeId,
        /// Whether trust was established.
        paired: bool,
        /// Short words both humans can compare, when it succeeded.
        verification: Option<String>,
        /// Why not, when it failed.
        detail: String,
    },

    /// An invitation for one machine to join this fleet.
    ///
    /// Carries what the operator needs to build a command on the other
    /// machine, and nothing else. The addresses are this node's own, so the
    /// joining machine knows where to dial back.
    JoinInvitation {
        /// Correlates the request.
        id: u32,
        /// The digits, or absent when the window was closed rather than opened.
        code: Option<String>,
        /// Where this node can be reached, best link first.
        addresses: Vec<String>,
        /// Seconds the invitation remains valid.
        expires_in_s: u64,
    },

    /// A cluster preview: the exact command each rank would run.
    ClusterPreview {
        /// Correlates the request.
        id: u32,
        /// One entry per rank, rank 0 first.
        ranks: Vec<RankPreview>,
        /// Warning about the fabric, when the plan would not run on RDMA.
        link_warning: Option<String>,
    },

    /// The outcome of a prepare across every rank.
    ClusterPrepared {
        /// Correlates the request.
        id: u32,
        /// Pins a later commit to this prepare.
        epoch: String,
        /// Per-rank outcome, rank 0 first.
        ranks: Vec<RankPrepare>,
        /// Whether every rank accepted, and a commit may therefore proceed.
        may_commit: bool,
    },

    /// Every rank of a cluster started.
    ClusterStarted {
        /// Correlates the request.
        id: u32,
        /// Which prepare this commit consumed.
        epoch: String,
        /// Per-rank outcome, rank 0 first.
        ranks: Vec<RankStarted>,
    },

    /// Every rank of a cluster was stopped.
    ClusterStopped {
        /// Correlates the request.
        id: u32,
        /// Ranks that were stopped, rank 0 first.
        ranks: Vec<RankStarted>,
    },

    /// How a running launch is doing.
    Stats {
        /// Correlates the request.
        id: u32,
        /// Which launch.
        recipe: RecipeId,
        /// The reading. Every field is optional: absent means the engine does
        /// not report it, or that there is not yet a second sample to
        /// difference against — never zero.
        stats: crate::msg::LaunchReading,
    },

    /// The tail of a launch's log.
    Logs {
        /// Correlates the request.
        id: u32,
        /// Which launch.
        recipe: RecipeId,
        /// The container the lines came from, so an operator can go read more.
        container: String,
        /// Lines, oldest first. Sanitised of control characters by the agent,
        /// because a log line is attacker-influenced text that a browser is
        /// about to render.
        lines: Vec<String>,
        /// Whether the container is still running. A tail from a container that
        /// has exited is the last thing it said, not the latest news.
        running: bool,
    },

    /// Something failed.
    Error {
        /// Correlation id, when the failure answers a request.
        id: Option<u32>,
        /// What went wrong.
        error: AgentError,
    },
}

/// One recipe, as the client sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeInfo {
    /// Stable identifier.
    pub id: RecipeId,
    /// Model this recipe serves.
    pub model: String,
    /// Nodes it needs.
    pub nodes: u32,
    /// Whether this agent can launch it.
    pub runnable: bool,
    /// Why not, when it cannot.
    pub reason: Option<String>,
    /// Settings the recipe sets, as launch defaults.
    pub defaults: BTreeMap<String, SettingValue>,
}

/// One reading of a running launch.
///
/// Every field optional, and absent is never zero: a dashboard that renders
/// 0 tok/s for "not measured yet" teaches an operator to distrust it, and the
/// one time throughput really is zero they will not believe the number.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LaunchReading {
    /// Requests served since the engine started.
    pub requests_total: Option<f64>,
    /// Requests in flight.
    pub requests_active: Option<f64>,
    /// Generated tokens per second over the sampling window.
    pub decode_tokens_per_s: Option<f64>,
    /// Prompt tokens per second over the sampling window.
    pub prompt_tokens_per_s: Option<f64>,
    /// Median time to first token, seconds.
    pub ttft_p50_s: Option<f64>,
    /// 90th percentile time to first token, seconds.
    pub ttft_p90_s: Option<f64>,
    /// Share of drafted tokens accepted, 0..1.
    pub accept_rate: Option<f64>,
    /// Share of prefix-cache lookups that hit, 0..1.
    pub prefix_hit_rate: Option<f64>,
    /// Seconds the rates cover, so the page can say how fresh they are rather
    /// than implying they are instantaneous.
    pub window_s: Option<f64>,
}

/// A launch currently running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunningLaunch {
    /// Container name.
    pub container: String,
    /// Recipe that produced it, when it is one of ours.
    pub recipe: Option<RecipeId>,
    /// Docker's status line.
    pub status: String,
}

pub mod fleet;

mod error;
pub use error::AgentError;

#[cfg(test)]
mod tests;
