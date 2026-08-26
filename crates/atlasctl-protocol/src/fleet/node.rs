// SPDX-License-Identifier: AGPL-3.0-only

//! What a node reports about itself: what it can do, how far it is trusted,
//! how it is, and what is wrong with it.
//!
//! Split from the identity and link vocabulary next door purely for size; the
//! rules are the same ones stated there, and the one that matters most lives
//! here: [`Metric`] keeps "cannot answer" distinct from "zero".

use super::{DisplayName, NodeAddress, NodeId};
use serde::{Deserialize, Serialize};

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
    /// Operating system, for display: `Linux`, `macOS`, `Windows`.
    ///
    /// Defaulted so a peer built before this field is understood as "did not
    /// say" rather than refused. Deliberately coarse and deliberately absent
    /// from the mDNS beacon: a kernel version on an unauthenticated broadcast
    /// is a shopping list for anyone listening.
    #[serde(default)]
    pub os: String,
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
