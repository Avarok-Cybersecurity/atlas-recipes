// SPDX-License-Identifier: AGPL-3.0-only

//! The message catalogue.
//!
//! Internally tagged on `type`, which maps directly onto a TypeScript
//! discriminated union so the browser narrows on `msg.type` without unwrapping
//! a nesting level.
//!
//! The set of client messages *is* the capability surface. There is no
//! raw-command verb, no nested-message verb, and no relay of opaque bytes,
//! and the enum is closed — an unknown `type` fails deserialization rather
//! than reaching a handler. One scoped exception exists and is stated here
//! so the code cannot outgrow its own doctrine: the seven single-node
//! control verbs carry an optional `on` target, which an agent honors by
//! re-issuing the request AS ITSELF over its authenticated peer channel —
//! one hop, only toward a machine the forwarding agent has itself pinned
//! AND whose pin of the requester carries the explicit `controller` grant,
//! and only within the closed [`ControlReq`] vocabulary, which cannot express
//! pairing, joining, cluster reservation, or a further hop. Forwarding is an
//! annotation on closed verbs, never a wrapper around arbitrary messages:
//! that is what still keeps the agent from becoming an open proxy for
//! whatever page is talking to it.

use crate::id::RecipeId;
use crate::settings::{SettingError, SettingValue};
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
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. Deliberately an annotation on a closed verb and
        /// NOT a `Forward { inner }` wrapper: a nesting slot is an open proxy
        /// waiting for one missed match arm, and an annotated verb keeps every
        /// forwardable operation individually visible in this enum, which is
        /// the capability surface. The six other single-node control verbs
        /// carry the same field with the same meaning.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
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
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
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
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
    },

    /// Stop a running launch.
    Stop {
        /// Correlates the reply.
        id: u32,
        /// Which recipe to stop.
        recipe: RecipeId,
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
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
    ///
    /// This runs the exchange and stops. **No pin is written**: the reply
    /// carries words for a human to compare, and trust is established only by
    /// [`Self::ConfirmPairing`]. Before protocol 2 the pin was written here and
    /// the words shown afterwards, which made comparing them a formality — a
    /// machine the operator went on to reject had already been trusted.
    PairPeer {
        /// Correlates the reply.
        id: u32,
        /// Which discovered peer.
        node: crate::fleet::NodeId,
        /// The digits the user read off the other machine.
        code: String,
    },

    /// Pair with a machine at an address the operator typed, rather than one
    /// that was discovered.
    ///
    /// Discovery is mDNS, which is link-local: it cannot cross a router, and it
    /// is switched off on plenty of managed networks. Without this the browser
    /// could only ever pair with machines on the same broadcast domain, and an
    /// operator who knows exactly where their machine is had no way to say so.
    /// The CLI has had `atlasctl peer add <host[:port]>` for this all along.
    ///
    /// Unlike [`Self::PairPeer`] there is no expected identity to check
    /// against: nothing has been discovered, so whoever answers that address is
    /// whoever answers. The reply therefore carries the identity that WAS
    /// reached, and the operator judges it at the same word-comparison step —
    /// which is the one place a human is already looking at who they are about
    /// to trust.
    PairPeerAt {
        /// Correlates the reply.
        id: u32,
        /// `host`, `host:port`, or `[v6]:port`. The peer port is assumed when
        /// none is given.
        target: String,
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
        /// Grant the peer being pinned the `controller` right (see
        /// `Pin::controller`) at the moment a human is already deciding to
        /// trust it. Defaults to false: consent to remote stop must be said,
        /// not implied by upgrading.
        #[serde(default)]
        allow_control: bool,
    },

    /// Trust a peer whose exchange has completed.
    ///
    /// Second half of [`Self::PairPeer`]. The exchange proves both machines
    /// derived the same key; this says a human compared the words and accepted
    /// them. Only now is a pin written, so a pairing the operator refuses never
    /// reaches disk rather than being written and then removed.
    ConfirmPairing {
        /// Correlates the reply.
        id: u32,
        /// The peer awaiting a decision.
        node: crate::fleet::NodeId,
        /// Grant the peer being pinned the `controller` right (see
        /// `Pin::controller`) at the moment a human is already deciding to
        /// trust it. Defaults to false: consent to remote stop must be said,
        /// not implied by upgrading.
        #[serde(default)]
        allow_control: bool,
    },

    /// Discard a completed exchange without trusting it.
    ///
    /// What the operator is saying is "those words did not match", which is the
    /// one thing the ceremony exists to detect. Nothing was written, so there is
    /// nothing to undo.
    RejectPairing {
        /// Correlates the reply.
        id: u32,
        /// The peer being refused.
        node: crate::fleet::NodeId,
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
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
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
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
    },

    /// Ask what is running.
    Status {
        /// Correlates the reply.
        id: u32,
        /// Which node this is for. `None` (and `Some(local id)`) means this
        /// machine, unchanged. See [`Self::ListRecipes`] for why this is an
        /// annotation rather than a wrapper.
        #[serde(default)]
        on: Option<crate::fleet::NodeId>,
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

// The agent's half. Split on the file-size cap, along the direction of travel
// rather than an arbitrary line.
mod server;
pub use server::ServerMsg;

mod error;
pub use error::AgentError;

// The forwardable control vocabulary: what one agent may relay to another.
// Its own file so the closed pair can be read — and reviewed — in isolation
// from the browser surface it mirrors.
mod control;
pub use control::{ControlRep, ControlReq};

#[cfg(test)]
mod tests;
