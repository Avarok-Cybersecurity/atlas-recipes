// SPDX-License-Identifier: AGPL-3.0-only

//! The event stream a job produces, and the artifacts it ends with.
//!
//! Events are journaled on the node with a monotonic `seq` starting at 1, so
//! a submitter that lost its connection re-attaches from the last seq it saw
//! and misses nothing. `Heartbeat` is the one kind that is never journaled:
//! it carries `seq == seq_high` and exists only to keep an attached stream
//! visibly alive.

use super::bench::{JobId, JobState, Sha};
use crate::fleet::NodeId;
use serde::{Deserialize, Serialize};

/// One event of one job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchEvent {
    pub job: JobId,
    /// 1-based, monotonic per job; `Heartbeat` repeats the latest.
    pub seq: u64,
    /// Node clock, unix milliseconds.
    pub at_ms: u64,
    #[serde(flatten)]
    pub kind: EventKind,
}

/// Which stream a log batch came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Build,
    Run,
}

/// The printed verdict of a gate run, as Atlas spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    Pass,
    Fail,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub text: String,
}

/// What kind of file an artifact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// A `.benchmarks/<gate>/*.json` gate record.
    Record,
    /// The record's `.json.sig` sidecar.
    Signature,
    /// The child's combined stdout/stderr.
    RunLog,
    /// Anything else the node was configured to collect.
    Extra,
}

/// One artifact, as listed and as fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// A bare file name, never a path.
    pub name: String,
    /// Where it belongs relative to the repo root, e.g.
    /// `.benchmarks/decode-floor/2026-09-13-1a0dc88a8c.json`.
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub kind: ArtifactKind,
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// The child ran to completion. `verdict` is what it printed; the exit
    /// code is recorded beside it, never instead of it.
    Completed {
        exit_code: i32,
        verdict: Option<Verdict>,
        /// The record's `relative_path`, when one was written.
        record: Option<String>,
        signature: Option<String>,
    },
    Failed {
        stage: JobState,
        reason: String,
    },
    TimedOut {
        stage: JobState,
        after_s: u64,
    },
    Cancelled {
        by: NodeId,
    },
    /// The agent restarted and could not recover the child.
    Orphaned {
        reason: String,
    },
}

impl Outcome {
    /// Did the child complete with a `Pass` verdict?
    #[must_use]
    pub fn passed(&self) -> bool {
        matches!(
            self,
            Self::Completed {
                verdict: Some(Verdict {
                    kind: VerdictKind::Pass,
                    ..
                }),
                ..
            }
        )
    }
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    Queued {
        position: u32,
    },
    Preparing {
        sha: Sha,
        /// Whether a fetch was needed to have the commit.
        fetched: bool,
    },
    /// The build decision. `cached` means the binary for this exact sha was
    /// reused; `reason` says why or why not.
    Build {
        cached: bool,
        reason: String,
    },
    Built {
        binary_sha256: String,
        cached: bool,
        secs: u64,
    },
    /// The node's own rendered argv, for the record.
    Running {
        pid: u32,
        argv: Vec<String>,
    },
    /// Best-effort phase, parsed from the child's output.
    Progress {
        phase: String,
        detail: String,
    },
    Log {
        stream: LogStream,
        lines: Vec<String>,
    },
    LogTruncated {
        dropped_bytes: u64,
    },
    /// Nested, not flattened: `Verdict.kind` would collide with this
    /// enum's own `kind` tag.
    Verdict {
        verdict: Verdict,
    },
    Artifact {
        meta: ArtifactMeta,
    },
    Done {
        #[serde(flatten)]
        outcome: Outcome,
    },
    /// Not journaled; `seq` repeats the latest.
    Heartbeat {
        state: JobState,
        seq_high: u64,
    },
}
