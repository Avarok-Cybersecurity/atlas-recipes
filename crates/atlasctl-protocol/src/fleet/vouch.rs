// SPDX-License-Identifier: AGPL-3.0-only

//! Second-hand fleet knowledge: what one pinned peer says about the peers it
//! has itself pinned.
//!
//! Nothing in this module carries trust. A digest entry is display and routing
//! data; pairing with a vouched node still requires the full ceremony against
//! that node directly.

use super::node::NodeVitals;
use super::{DisplayName, LinkClass, NodeAddress, NodeId};
use serde::{Deserialize, Serialize};

/// One entry in a fleet digest: a first-person statement by the SPEAKER —
/// "I have pinned this node; this is what it told me over my authenticated
/// channel, and the link I reach it on."
///
/// Every field is a serialization of what the speaker's pin store and live
/// `PeerReport` cache already hold (SSOT); nothing here is invented, and
/// nothing here may come from a digest the speaker itself received —
/// knowledge, like forwarding, is one hop by construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VouchedPeer {
    /// The vouched node's identity — a key fingerprint. Display and routing
    /// data ONLY: a receiver never writes this to a pin store, so a fabricated
    /// entry fails the ceremony the moment anyone actually pairs with it.
    pub node: NodeId,
    /// Its display name as it stated it. Hostile text; re-sanitised on read.
    pub name: DisplayName,
    /// Whether it says it can run a model.
    pub can_launch: bool,
    /// Coarse accelerator tag, as it stated it.
    pub accelerator: String,
    /// Coarse OS, as it stated it.
    pub os: String,
    /// Addresses IT claimed over the speaker's authenticated channel.
    pub addresses: Vec<NodeAddress>,
    /// The SPEAKER's classification of the SPEAKER's leg to it — the link a
    /// forwarded request would actually ride. Consistent with the existing
    /// rule that a link opinion describes only paths the speaker is on.
    pub link: LinkClass,
    /// Whether the speaker's last poll of it succeeded.
    pub reachable: bool,
    /// Its last vitals as held by the speaker, with the age below. Both or
    /// neither: vitals without an age would render second-hand data as fresh.
    pub vitals: Option<NodeVitals>,
    /// Seconds since the speaker recorded those vitals. Absent means the
    /// vitals are absent too (PCND: an unknown age is not zero). Receivers
    /// drop vitals whose age exceeds `UNREACHABLE_AFTER` rather than invent
    /// a new threshold.
    pub vitals_age_s: Option<u64>,
}

/// Most entries a digest may carry, enforced by builder AND accepter.
/// A speaker exceeding it is misbehaving, so an oversized digest is refused
/// wholesale — truncating it would silently hide fleet members. The builder
/// emits entries in `NodeId` byte order so which 64 is deterministic.
pub const MAX_VOUCHED: usize = 64;
