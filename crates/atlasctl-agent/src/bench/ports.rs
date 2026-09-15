// SPDX-License-Identifier: AGPL-3.0-only

//! What a job's life needs from the machine it runs on, as a trait, and the
//! values that cross it. [`machine`](super::machine) is the procedure over
//! these; [`ports_std`](super::ports_std) is the Linux implementation; the
//! machine's tests script one.

use super::job::JobStore;
use super::journal::Journal;
use anyhow::Result;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::Sha;
use atlasctl_protocol::msg::bench_event::{ArtifactMeta, EventKind, Verdict};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// A binary already built for a sha, verified against its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBinary {
    pub path: PathBuf,
    pub sha256: String,
}

/// Why a cache lookup did not hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheMiss {
    NoBuild,
    NoProvenance,
    /// Provenance names another commit (a copied directory, a rename).
    WrongSha(String),
    /// The bytes on disk do not hash to what provenance recorded.
    HashMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildResult {
    pub binary: CachedBinary,
    pub secs: u64,
}

/// A spawned child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildHandle {
    pub pid: u32,
    pub start_ticks: u64,
}

/// How the run phase ended.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEnd {
    Exited {
        code: i32,
        verdict: Option<Verdict>,
    },
    /// No output for the stall timeout.
    Stalled {
        after_s: u64,
    },
    /// Past the run ceiling.
    TimedOut {
        after_s: u64,
    },
    Cancelled,
}

/// The parameters of one run, rendered by the node.
pub struct RunPlan {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub log_path: PathBuf,
    pub stall_timeout: Duration,
    pub max_run: Duration,
}

/// Everything the lifecycle asks of the world.
pub trait Ports: Send + Sync {
    fn now_ms(&self) -> u64;
    fn has_commit(&self, sha: &Sha) -> Result<bool>;
    fn fetch(&self) -> Result<()>;
    fn published(&self, sha: &Sha) -> Result<bool>;
    /// Create or verify the worktree for `sha`, returning its path.
    fn worktree(&self, sha: &Sha) -> Result<PathBuf>;
    fn cached_binary(&self, sha: &Sha) -> Result<Result<CachedBinary, CacheMiss>>;
    fn build(&self, sha: &Sha, worktree: &Path, cancel: &AtomicBool) -> Result<BuildResult>;
    /// Recipes present in the home, or sync them; `Ok(true)` when a sync ran.
    fn ensure_recipes(&self, binary: &Path) -> Result<bool>;
    /// The gates this binary knows, for a refusal that names them.
    fn known_gates(&self, binary: &Path) -> Result<Vec<String>>;
    /// Parameter names the gate does not accept.
    fn bad_params(&self, binary: &Path, gate: &str, params: &[String]) -> Result<Vec<String>>;
    fn spawn(&self, plan: &RunPlan) -> Result<ChildHandle>;
    /// Wait for the child, emitting log/progress events as they appear;
    /// honours `cancel`.
    fn wait(
        &self,
        child: &ChildHandle,
        plan: &RunPlan,
        cancel: &AtomicBool,
        emit: &dyn Fn(EventKind),
    ) -> Result<RunEnd>;
    /// The gate's new records and any extras, copied into `dest`.
    fn collect(
        &self,
        worktree: &Path,
        gate: &str,
        since_s: u64,
        dest: &Path,
    ) -> Result<Vec<ArtifactMeta>>;
}

/// The node running this job, for `Cancelled { by }`.
pub struct Ctx<'a> {
    pub ports: &'a dyn Ports,
    pub store: &'a JobStore,
    pub journal: &'a Journal,
    pub local: NodeId,
    pub cancel: Arc<AtomicBool>,
    pub hardware: &'a str,
    pub child_env: Vec<(String, String)>,
    pub build_timeout: Duration,
    pub stall_timeout: Duration,
    pub allow_unpublished: bool,
    /// Render `--serve-reuse --serve-lease-owner <agent pid>` into the run.
    pub serve_reuse: bool,
}
