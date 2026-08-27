// SPDX-License-Identifier: AGPL-3.0-only

//! What the agent says.
//!
//! Split from `msg.rs` on the 500-line cap, along the direction of travel: this
//! file is everything the agent sends, its sibling everything a client asks.

use super::{AgentError, RecipeInfo, RunningLaunch};
use crate::id::RecipeId;
use crate::msg::fleet::{FleetEvent, RankPrepare, RankPreview, RankStarted};
use crate::settings::SettingSpec;
use serde::{Deserialize, Serialize};

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
        /// Which node's answer this is. `None` = this machine. The server
        /// states it rather than letting the page infer it from its own
        /// request, so a misrouted reply is visible instead of silently
        /// misattributed.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel. A reply with `via: Some(r)` is
        /// exactly as trustworthy as `r`, and the page must say so. The six
        /// other single-node replies carry the same pair with the same
        /// meaning.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
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
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
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
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
    },

    /// Reply to `Status`.
    Status {
        /// Correlation id.
        id: u32,
        /// What is running.
        running: Vec<RunningLaunch>,
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
    },

    /// A launch was stopped.
    Stopped {
        /// Correlation id.
        id: u32,
        /// Which recipe.
        recipe: RecipeId,
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
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
    /// The outcome of pairing with a typed address.
    ///
    /// Separate from [`Self::PairResult`] because the node is not known up
    /// front and may never be known: if nothing answers the address, there is
    /// no identity to name. `PairResult` requires one, and filling it with a
    /// zero id to satisfy the type would be a claim about a machine that was
    /// never reached.
    PairAtResult {
        /// Correlates the request.
        id: u32,
        /// Who answered. `None` when nothing did.
        node: Option<crate::fleet::NodeId>,
        /// What that machine calls itself, for the operator to weigh alongside
        /// the words. Empty when nothing answered.
        name: String,
        /// Where it was reached. Empty when nothing answered.
        address: String,
        /// Whether the exchange completed.
        ///
        /// Invariant, as for `PairResult`: `exchanged == verification.is_some()`.
        exchanged: bool,
        /// Words for the two humans to compare, when the exchange completed.
        verification: Option<String>,
        /// Why not, when it did not.
        detail: String,
    },

    PairResult {
        /// Correlates the request.
        id: u32,
        /// The peer.
        node: crate::fleet::NodeId,
        /// Whether the exchange completed and both sides derived the same key.
        ///
        /// Named `exchanged` rather than `paired` because it is not trust: no
        /// pin exists yet. The rename is deliberate — the field used to mean
        /// "trusted", and a client still reading it that way would show a
        /// machine as paired that this agent has not accepted. Protocol 2
        /// refuses such a client at the handshake.
        ///
        /// Invariant: `exchanged == verification.is_some()`.
        exchanged: bool,
        /// Words for the two humans to compare, when the exchange completed.
        verification: Option<String>,
        /// Why not, when it did not.
        detail: String,
    },

    /// The outcome of a trust decision: confirm, reject, or unpair.
    ///
    /// Separate from [`Self::PairResult`] because it answers a different
    /// question. `PairResult` reports what a ceremony did; this reports what is
    /// now true about trust.
    PairDecision {
        /// Correlates the request.
        id: u32,
        /// The peer.
        node: crate::fleet::NodeId,
        /// Whether this agent trusts the peer now.
        trusted: bool,
        /// What happened, in words the operator can act on.
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
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
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
        /// Which node's answer this is. `None` = this machine. See
        /// [`Self::Recipes`] for why the server states it.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
        /// The relay that carried it, when one did. `None` = answered over a
        /// direct authenticated channel.
        #[serde(default)]
        via: Option<crate::fleet::NodeId>,
    },

    /// Something failed.
    Error {
        /// Correlation id, when the failure answers a request.
        id: Option<u32>,
        /// What went wrong.
        error: AgentError,
    },
}
