// SPDX-License-Identifier: AGPL-3.0-only

//! What a node reports about itself to a bench submitter.
//!
//! Facts only. Whether two nodes are "equivalent enough" to split a
//! speed-class measurement across is decided by the submitter (Atlas decides
//! it from these fields and from the records themselves, so its CI reaches the
//! same answer); a node states what it is and never what it is like another.

use super::bench::{JobSummary, Sha};
use crate::fleet::{DisplayName, Metric, NodeAlert, NodeId};
use serde::{Deserialize, Serialize};

/// The accelerator, as `nvidia-smi` and the telemetry sampler describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuInfo {
    /// e.g. `NVIDIA GB10`.
    pub name: String,
    pub count: u32,
    pub driver_version: String,
    /// From the `nvidia-smi` header, e.g. `13.0`; empty when unknown.
    pub cuda_version: String,
    pub sm_clock_mhz: Metric,
    pub sm_clock_healthy_mhz: Option<u32>,
    pub temperature_c: Metric,
    pub memory_total_bytes: Metric,
    pub memory_used_frac: Metric,
    pub memory_is_unified: bool,
}

/// The live thermal and capacity facts a speed-class equivalence check
/// reads: the two GB10s that read 0.66 tok/s apart differed in nothing
/// static — only in these.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HostThermal {
    /// Every `/sys/class/thermal/thermal_zone*` reading, °C, in zone order.
    #[serde(default)]
    pub chassis_temps_c: Vec<f64>,
    /// Whether any THERMAL clock-event reason is asserted right now
    /// (`SW Thermal Slowdown`, `HW Thermal Slowdown`, `HW Power Braking`);
    /// `None` when `nvidia-smi -q -d PERFORMANCE` did not answer.
    #[serde(default)]
    pub throttle_thermal: Option<bool>,
    /// The part's SM clock ceiling, MHz (`clocks.max.sm`).
    #[serde(default)]
    pub sm_clock_max_mhz: Option<f64>,
    /// `MemTotal` from `/proc/meminfo`, kB.
    #[serde(default)]
    pub mem_total_kb: Option<u64>,
}

/// The Atlas checkout the node builds from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub path: String,
    pub remote_name: String,
    pub remote_url: String,
    pub head_sha: Option<Sha>,
    pub fetched_at_s: Option<u64>,
}

/// A commit whose `spark` is already built on this node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuiltSha {
    pub sha: Sha,
    pub binary_sha256: String,
    pub built_at_s: u64,
    pub bytes: u64,
}

/// The whole report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchNodeInfo {
    pub node: NodeId,
    pub name: DisplayName,
    pub agent_version: String,
    pub peer_version_max: u32,
    pub bench_enabled: bool,
    pub disabled_reason: Option<String>,
    pub gpu: Option<GpuInfo>,
    /// Live thermal and capacity facts; absent on a node that cannot read
    /// them, which an equivalence check treats as "not the same box".
    #[serde(default)]
    pub thermal: Option<HostThermal>,
    /// Live alerts (clock clamped, thermal throttle, memory pressure, …).
    pub alerts: Vec<NodeAlert>,
    /// The configured box class the records will name, e.g. `gb10`.
    pub hardware_class: Option<String>,
    pub atlas_repo: Option<RepoInfo>,
    pub atlas_home: Option<String>,
    /// First 16 hex of SHA-256 of the signing public key, as Atlas spells it.
    pub signer_fp: Option<String>,
    pub signer_pubkey_hex: Option<String>,
    pub recipes_synced: bool,
    pub built_shas: Vec<BuiltSha>,
    /// Something exclusive is using the box right now.
    pub busy: bool,
    pub busy_reason: Option<String>,
    pub running_job: Option<JobSummary>,
    pub queued: u32,
    pub queue_depth: u32,
    pub disk_free_bytes: Metric,
    pub min_free_disk_bytes: u64,
    pub min_free_fraction: f64,
    /// `MemAvailable / MemTotal` right now.
    pub host_free_fraction: Metric,
    pub max_run_s: u32,
}
