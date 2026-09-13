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
