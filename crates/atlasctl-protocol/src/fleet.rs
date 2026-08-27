// SPDX-License-Identifier: AGPL-3.0-only

//! The vocabulary for a fleet: nodes, the links between them, and what each
//! node is willing to say about itself.
//!
//! Two rules shape every type here.
//!
//! **Identity is a key, never a name.** DGX Sparks ship with hostnames like
//! `spark-256a` and `spark-43fa`; they collide, they change, and they arrive
//! over an unauthenticated multicast beacon. A [`NodeId`] is the fingerprint of
//! an Ed25519 public key, so two nodes are the same node when they can prove
//! the same private key, and for no other reason. Hostnames are display-only
//! and [`DisplayName`] length-caps and sanitises them, because they are
//! attacker-controlled text that a public website renders.
//!
//! **Absent is not zero.** Every measurement is a [`Metric`], which is either a
//! reading or an explicit statement that this hardware cannot answer. On a
//! GB10 the GPU memory fields are genuinely unanswerable — Grace-Blackwell is
//! unified memory — and a dashboard that rendered `0 GB` there would be
//! reporting a measurement it never took.

use serde::{Deserialize, Serialize};
use std::fmt;

/// How long a display string from an untrusted source may be.
const DISPLAY_MAX: usize = 63;

/// A node's stable identity: the SHA-256 fingerprint of its Ed25519 public key.
///
/// Parse-don't-validate — once you hold one of these it is 32 bytes of real
/// fingerprint, not a string someone sent you.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NodeId([u8; 32]);

impl NodeId {
    /// Wrap raw fingerprint bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw fingerprint.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The form a human compares out loud during pairing: four groups of four
    /// hex digits from the front of the fingerprint. Short enough to read over
    /// a desk, long enough that forging a match is not a practical exercise.
    #[must_use]
    pub fn short(&self) -> String {
        let hex = self.to_string();
        let mut out = String::with_capacity(19);
        for (i, chunk) in hex.as_bytes().chunks(4).take(4).enumerate() {
            if i > 0 {
                out.push('-');
            }
            out.push_str(std::str::from_utf8(chunk).unwrap_or("????"));
        }
        out
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl From<NodeId> for String {
    fn from(id: NodeId) -> Self {
        id.to_string()
    }
}

/// Why a string was not a node id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NodeIdError {
    /// Wrong length.
    #[error("a node id is 64 hex characters, got {0}")]
    Length(usize),
    /// Not hex.
    #[error("a node id is hexadecimal; byte {0} is not")]
    NotHex(usize),
}

impl TryFrom<String> for NodeId {
    type Error = NodeIdError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl NodeId {
    /// Parse a 64-character hex fingerprint.
    ///
    /// # Errors
    /// If the string is not exactly 64 hexadecimal characters.
    pub fn parse(s: &str) -> Result<Self, NodeIdError> {
        if s.len() != 64 {
            return Err(NodeIdError::Length(s.len()));
        }
        let mut out = [0u8; 32];
        for (i, pair) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_val(pair[0]).ok_or(NodeIdError::NotHex(i * 2))?;
            let lo = hex_val(pair[1]).ok_or(NodeIdError::NotHex(i * 2 + 1))?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

const fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A display string that arrived from an untrusted source.
///
/// A beacon is an unauthenticated write into someone's UI, so a hostname is
/// sanitised and length-capped at the boundary rather than at every render
/// site. Control characters are dropped outright; nothing here is HTML-escaped,
/// because escaping belongs to the renderer and doing it twice is how `&amp;`
/// ends up on screen.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct DisplayName(String);

impl DisplayName {
    /// Sanitise and cap an arbitrary string.
    #[must_use]
    pub fn new(raw: &str) -> Self {
        let cleaned: String = raw
            .chars()
            .filter(|c| !c.is_control())
            .take(DISPLAY_MAX)
            .collect();
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            Self("unnamed".to_owned())
        } else {
            Self(trimmed.to_owned())
        }
    }

    /// The safe string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for DisplayName {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl From<DisplayName> for String {
    fn from(d: DisplayName) -> Self {
        d.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What kind of link an address sits on.
///
/// This drives a warning, not a launch decision. EP=2 decode is all-reduce
/// bound, so a cluster that falls back to ethernet runs several times slower
/// while every correctness check still passes — exactly the kind of silent loss
/// that has to be visible in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkClass {
    /// RDMA over converged ethernet. What the published numbers were measured on.
    Roce,
    /// Native InfiniBand.
    InfiniBand,
    /// Ordinary wired ethernet. Works, and is much slower for collectives.
    Ethernet,
    /// Wireless. Usable for control, never for a collective.
    Wireless,
    /// A bridge, veth, dummy or other software interface.
    Virtual,
    /// Loopback.
    Loopback,
    /// Not established. A peer's beacon says where it is but is
    /// unauthenticated, so its claim about the link is not evidence — and
    /// guessing "ethernet" would tell someone their 200 Gb RoCE fabric is slow.
    /// Resolved once the peer is reached over the authenticated channel.
    Unverified,
}

impl LinkClass {
    /// Preference order for carrying collective traffic. Higher is better.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::InfiniBand => 5,
            Self::Roce => 4,
            Self::Ethernet => 3,
            Self::Wireless => 2,
            // Below anything known: an unverified link is a candidate, never a
            // preference.
            Self::Unverified => 1,
            Self::Virtual | Self::Loopback => 0,
        }
    }

    /// Whether a cluster may be formed over this link at all.
    #[must_use]
    pub const fn usable_for_cluster(self) -> bool {
        matches!(
            self,
            Self::InfiniBand | Self::Roce | Self::Ethernet | Self::Unverified
        )
    }

    /// Whether another machine could dial this one over this link.
    ///
    /// Deliberately NOT [`Self::usable_for_cluster`]. That question is "can
    /// this link carry a collective?", and it answers no for wireless — which
    /// is right for collectives and wrong for reaching a machine at all. A
    /// pairing dial-back is control traffic, and this enum's own documentation
    /// says wireless is "usable for control, never for a collective".
    ///
    /// Using the collective predicate here meant a laptop on Wi-Fi advertised
    /// NO address in a join invitation, so the page built an empty command and
    /// rendered an empty box with a Copy button beside it. That laptop is the
    /// exact machine the invitation exists for: it cannot run models, so it
    /// invites one that can.
    ///
    /// Loopback and virtual links stay excluded — they are reachable from here
    /// and from nowhere else, so naming one produces a command that installs
    /// cleanly on the far machine and then cannot dial back.
    #[must_use]
    pub const fn usable_for_control(self) -> bool {
        !matches!(self, Self::Loopback | Self::Virtual)
    }

    /// Whether using this link should raise a visible warning.
    #[must_use]
    pub const fn warns(self) -> bool {
        // Unverified does not warn: it is an absence of information, and
        // dressing that up as a problem is its own kind of wrong.
        !matches!(self, Self::InfiniBand | Self::Roce | Self::Unverified)
    }

    /// Short human label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Roce => "RoCE",
            Self::InfiniBand => "InfiniBand",
            Self::Ethernet => "Ethernet",
            Self::Wireless => "Wi-Fi",
            Self::Virtual => "virtual",
            Self::Loopback => "loopback",
            Self::Unverified => "link unverified",
        }
    }
}

/// One address a node can be reached on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeAddress {
    /// Interface it belongs to, for display and for `NCCL_SOCKET_IFNAME`.
    pub iface: String,
    /// The address itself.
    pub addr: String,
    /// Prefix length of the subnet it sits on.
    ///
    /// Kept, not discarded. A DGX Spark carries four RoCE ports on separate
    /// point-to-point /30s, and which of them reaches a given peer is decided
    /// entirely by this number — a host address alone cannot answer it. The
    /// first version of this struct dropped the prefix on the grounds that a
    /// peer address is a host address, and the result was a cluster that chose
    /// a rendezvous address on a link the worker was not attached to and hung
    /// at the NCCL barrier until somebody read the logs.
    ///
    /// Defaulted for the wire so an older peer that does not send it is
    /// understood as "unknown subnet" rather than refused.
    #[serde(default)]
    pub prefix_len: u8,
    /// What kind of link this is.
    pub class: LinkClass,
    /// Negotiated speed in Mb/s, when the kernel reports one.
    pub speed_mbps: Option<u32>,
    /// Whether an RDMA device is bound to this interface.
    pub rdma: bool,
}

pub mod node;
pub mod vouch;

pub use node::{
    AlertKind, Launchability, Metric, NodeAlert, NodeDescriptor, NodeVitals, PairingState, Severity,
};
pub use vouch::{MAX_VOUCHED, VouchedPeer};

#[cfg(test)]
mod tests;
