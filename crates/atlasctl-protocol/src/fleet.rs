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
            Self::Virtual | Self::Loopback => 0,
        }
    }

    /// Whether a cluster may be formed over this link at all.
    #[must_use]
    pub const fn usable_for_cluster(self) -> bool {
        matches!(self, Self::InfiniBand | Self::Roce | Self::Ethernet)
    }

    /// Whether using this link should raise a visible warning.
    #[must_use]
    pub const fn warns(self) -> bool {
        !matches!(self, Self::InfiniBand | Self::Roce)
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
    /// What kind of link this is.
    pub class: LinkClass,
    /// Negotiated speed in Mb/s, when the kernel reports one.
    pub speed_mbps: Option<u32>,
    /// Whether an RDMA device is bound to this interface.
    pub rdma: bool,
}

/// Whether this machine can run a model, and if not, why not.
///
/// A node that cannot launch is still worth having in the fleet: it can watch,
/// pair, and drive other nodes. Windows, macOS, and any agent started with
/// `--client` report `false` through this one mechanism rather than each having
/// its own special case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launchability {
    /// Whether a launch would be accepted.
    pub can_launch: bool,
    /// Why not, in words a user can act on. Empty when it can.
    pub reason: String,
}

impl Launchability {
    /// A node that can run models.
    #[must_use]
    pub fn yes() -> Self {
        Self {
            can_launch: true,
            reason: String::new(),
        }
    }

    /// A node that cannot, and the reason.
    #[must_use]
    pub fn no(reason: impl Into<String>) -> Self {
        Self {
            can_launch: false,
            reason: reason.into(),
        }
    }
}

/// How far a peer has got through the trust ceremony.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingState {
    /// Seen on the network. Carries no authority whatsoever.
    Discovered,
    /// A pairing attempt is in progress.
    Pairing,
    /// Key pinned. This peer can run models on your hardware.
    Paired,
    /// Was paired, and is not answering.
    Unreachable,
}

/// A reading, or an explicit statement that this hardware cannot answer.
///
/// The `Unsupported` arm is the whole point: it lets a dashboard say "not
/// available on this hardware" instead of drawing a zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Metric {
    /// A real reading.
    Reading {
        /// The value.
        value: f64,
    },
    /// This machine cannot report this. Not an error, and not a zero.
    Unsupported,
}

impl Metric {
    /// A reading.
    #[must_use]
    pub const fn reading(value: f64) -> Self {
        Self::Reading { value }
    }

    /// The value, if there is one.
    #[must_use]
    pub const fn value(self) -> Option<f64> {
        match self {
            Self::Reading { value } => Some(value),
            Self::Unsupported => None,
        }
    }
}

impl From<Option<f64>> for Metric {
    fn from(v: Option<f64>) -> Self {
        v.map_or(Self::Unsupported, |value| Self::Reading { value })
    }
}

/// What a node reports about its own health, running a model or not.
///
/// Idle vitals are the useful ones: a box with a clamped clock, a failing fan
/// or a full cache filesystem is something you want to know about *before* you
/// launch on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeVitals {
    /// Accelerator utilisation, percent.
    pub accelerator_util: Metric,
    /// SM clock in MHz.
    pub sm_clock_mhz: Metric,
    /// Clock below which, under load, the part is considered clamped.
    pub sm_clock_healthy_mhz: Option<u32>,
    /// Accelerator temperature, Celsius.
    pub temperature_c: Metric,
    /// Power draw, watts.
    pub power_w: Metric,
    /// Fraction of unified memory in use, 0..1.
    pub memory_used_frac: Metric,
    /// Total unified memory, bytes.
    pub memory_total_bytes: Metric,
    /// Free space on the filesystem holding images and the model cache, bytes.
    pub disk_free_bytes: Metric,
    /// Whether the container runtime answered.
    pub docker_ok: bool,
    /// Seconds since the agent started.
    pub agent_uptime_s: u64,
}

impl Default for NodeVitals {
    fn default() -> Self {
        Self {
            accelerator_util: Metric::Unsupported,
            sm_clock_mhz: Metric::Unsupported,
            sm_clock_healthy_mhz: None,
            temperature_c: Metric::Unsupported,
            power_w: Metric::Unsupported,
            memory_used_frac: Metric::Unsupported,
            memory_total_bytes: Metric::Unsupported,
            disk_free_bytes: Metric::Unsupported,
            docker_ok: false,
            agent_uptime_s: 0,
        }
    }
}

/// Something about a node that a person should look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    /// SM clock is pinned low while the part is busy. This one has cost us
    /// whole benchmark campaigns: a clamped clock makes every throughput
    /// number 2.5-2.9x low while every correctness gate still passes.
    SmClockClamped,
    /// Thermal throttling.
    ThermalThrottle,
    /// Unified memory nearly exhausted.
    MemoryPressure,
    /// Disk headroom low on the image or cache filesystem — a leading cause of
    /// launch failure.
    DiskLow,
    /// The container runtime is not answering.
    DockerUnreachable,
    /// A paired peer stopped responding.
    PeerLost,
    /// A container is restarting in a loop.
    RestartLoop,
}

impl AlertKind {
    /// Short label for a badge.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::SmClockClamped => "clock clamped",
            Self::ThermalThrottle => "thermal throttle",
            Self::MemoryPressure => "memory pressure",
            Self::DiskLow => "disk low",
            Self::DockerUnreachable => "docker unreachable",
            Self::PeerLost => "peer lost",
            Self::RestartLoop => "restart loop",
        }
    }
}

/// How much attention an alert deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Worth knowing.
    Info,
    /// Something is degraded but working.
    Warning,
    /// Something is broken.
    Critical,
}

/// A raised alert.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeAlert {
    /// What kind.
    pub kind: AlertKind,
    /// How bad.
    pub severity: Severity,
    /// Human detail, already safe to render.
    pub detail: String,
}

/// Everything the browser needs to draw one node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDescriptor {
    /// Stable identity.
    pub id: NodeId,
    /// Hostname, sanitised.
    pub name: DisplayName,
    /// Whether this is the node the browser is talking to.
    pub is_local: bool,
    /// Trust state.
    pub pairing: PairingState,
    /// Addresses, best link first.
    pub addresses: Vec<NodeAddress>,
    /// Whether it can run a model.
    pub launchability: Launchability,
    /// Agent version string.
    pub agent_version: String,
    /// Coarse accelerator description, for display.
    pub accelerator: String,
    /// Latest vitals, when the node has reported any.
    pub vitals: Option<NodeVitals>,
    /// Active alerts.
    pub alerts: Vec<NodeAlert>,
    /// Recipe currently running on this node, if any.
    pub running: Option<String>,
}

impl NodeDescriptor {
    /// The address a collective should use, which is the best usable link.
    #[must_use]
    pub fn preferred_address(&self) -> Option<&NodeAddress> {
        self.addresses
            .iter()
            .filter(|a| a.class.usable_for_cluster())
            .max_by_key(|a| (a.class.rank(), a.speed_mbps.unwrap_or(0)))
    }
}

#[cfg(test)]
mod tests;
