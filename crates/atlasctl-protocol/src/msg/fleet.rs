// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet-shaped payloads carried by [`super::ServerMsg`].
//!
//! Split out of `msg.rs` to keep that file readable rather than for any
//! structural reason: these are the same closed, internally-tagged shapes as
//! the rest of the protocol.

use crate::fleet::{AlertKind, DisplayName, NodeAlert, NodeDescriptor, NodeId, NodeVitals};
use serde::{Deserialize, Serialize};

/// What one rank would run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankPreview {
    /// Which node.
    pub node: NodeId,
    /// Its display name.
    pub name: DisplayName,
    /// Rank number; 0 is the head.
    pub rank: u16,
    /// The address every rank rendezvouses on.
    pub master_addr: String,
    /// The command, shell-quoted for reading and copying.
    pub command: String,
    /// Values this machine's flag table does not claim, and which therefore
    /// reach nothing.
    ///
    /// Reported per rank rather than once, because the machines can be running
    /// different revisions: a value that lands on rank 0 can be silently
    /// dropped on rank 1, and that asymmetry is exactly what an operator needs
    /// to see. The tool this replaces discarded these without a word, which is
    /// how a recipe's stated correctness pin went unapplied for months.
    pub unmapped: Vec<String>,
}

/// One rank's answer to a prepare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankPrepare {
    /// Which node.
    pub node: NodeId,
    /// Its display name.
    pub name: DisplayName,
    /// Rank number.
    pub rank: u16,
    /// Whether it is ready.
    pub prepared: bool,
    /// Why not, when it refused.
    pub reason: String,
}

/// One rank that started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankStarted {
    /// Which node.
    pub node: NodeId,
    /// Its display name.
    pub name: DisplayName,
    /// Rank number.
    pub rank: u16,
    /// Container id on that machine.
    pub container: String,
    /// Where the API is, for rank 0. Worker ranks serve nothing and carry
    /// `None` rather than a URL that would not answer.
    pub endpoint: Option<String>,
}

/// Something that happened to the fleet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum FleetEvent {
    /// A node appeared or its description changed.
    NodeChanged {
        /// The new description.
        node: Box<NodeDescriptor>,
    },
    /// A node has not been heard from and is now considered gone.
    ///
    /// Raised only after a node has been missing across two discovery
    /// intervals, so a single missed refresh on busy wifi does not make it
    /// blink out of the interface.
    NodeGone {
        /// Which node.
        node: NodeId,
    },
    /// Fresh vitals for a node.
    Vitals {
        /// Which node.
        node: NodeId,
        /// The sample.
        vitals: Box<NodeVitals>,
    },
    /// An alert was raised.
    AlertRaised {
        /// Which node.
        node: NodeId,
        /// The alert.
        alert: NodeAlert,
    },
    /// An alert cleared.
    AlertCleared {
        /// Which node.
        node: NodeId,
        /// Which kind cleared.
        kind: AlertKind,
    },
}
