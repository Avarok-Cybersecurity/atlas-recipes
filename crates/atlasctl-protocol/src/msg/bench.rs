// SPDX-License-Identifier: AGPL-3.0-only

//! The benchmark-job vocabulary: what a `bench`-granted peer may ask a node
//! to do, and what the node answers.
//!
//! A **closed enum, like [`ControlReq`](super::ControlReq)**. Nothing here
//! carries an argv, an environment, a path or a URL: a submitter names a
//! commit and a gate, both as validated newtypes, and the node renders the
//! command from its own configuration. What a `bench` grant consents to is
//! therefore "build and run the Atlas checkout I configured, at a commit my
//! trusted remote already has, and hand back the records it writes" — and
//! nothing a filter could miss.
//!
//! Jobs run for hours and the submitter's connection is expected to drop, so
//! submission carries a client-chosen [`JobKey`] and the node's job store is
//! the source of truth: resubmitting the same key returns the same job. That
//! is a deliberate departure from `control.rs`, whose `Launch` has no
//! idempotency key because a launch answers within its budget.

use crate::fleet::NodeId;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Largest artifact chunk the node sends in one frame. Base64 of this plus the
/// envelope stays under the 1 MiB peer frame cap.
pub const MAX_CHUNK: u32 = 512 * 1024;
/// Most lines one `Log` event carries.
pub const MAX_LOG_LINES_PER_EVENT: usize = 64;
/// Longest sanitised log line kept.
pub const MAX_LOG_LINE_BYTES: usize = 2000;
/// Most parameters a submission may carry.
pub const MAX_PARAMS: usize = 32;
pub const MAX_PARAM_KEY: usize = 64;
pub const MAX_PARAM_VALUE: usize = 256;
pub const MAX_NOTE: usize = 200;
/// Largest single artifact a job may hand back.
pub const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
/// Largest total per job.
pub const MAX_JOB_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

/// Why a newtype refused its input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BenchIdError {
    #[error("empty")]
    Empty,
    #[error("{0} bytes, more than {1}")]
    Length(usize, usize),
    #[error("character {0:?} is not allowed")]
    Charset(char),
    #[error("must be exactly 40 lowercase hex digits")]
    NotASha,
    #[error("must not start with `-`")]
    FlagShaped,
}

fn check_token(s: &str, max: usize, extra: &[char]) -> Result<(), BenchIdError> {
    if s.is_empty() {
        return Err(BenchIdError::Empty);
    }
    if s.len() > max {
        return Err(BenchIdError::Length(s.len(), max));
    }
    if s.starts_with('-') {
        return Err(BenchIdError::FlagShaped);
    }
    if let Some(c) = s
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || extra.contains(c)))
    {
        return Err(BenchIdError::Charset(c));
    }
    Ok(())
}

macro_rules! token_newtype {
    ($(#[$doc:meta])* $name:ident, $max:expr, $extra:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        pub struct $name(String);

        impl $name {
            /// Validate.
            pub fn parse(s: &str) -> Result<Self, BenchIdError> {
                check_token(s, $max, $extra)?;
                Ok(Self(s.to_string()))
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(d)?;
                Self::parse(&raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

token_newtype!(
    /// The submitter's idempotency key: `[A-Za-z0-9._-]{1,64}`.
    JobKey,
    64,
    &['.', '_', '-']
);
token_newtype!(
    /// The node-minted job identifier, `jb-<unix>-<8hex>`.
    JobId,
    64,
    &['-']
);
token_newtype!(
    /// A benchmark id as the Atlas registry spells it.
    GateId,
    64,
    &['.', '_', '-']
);

/// A full 40-hex-digit commit id. Short forms are refused: a job pins a tree,
/// and a prefix is not a tree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Sha(String);

impl Sha {
    pub fn parse(s: &str) -> Result<Self, BenchIdError> {
        if s.len() != 40 || !s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            return Err(BenchIdError::NotASha);
        }
        Ok(Self(s.to_string()))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The first ten digits, for logs.
    #[must_use]
    pub fn short(&self) -> &str {
        &self.0[..10]
    }
}

impl fmt::Display for Sha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Validate a parameter map against the size bounds. Values are opaque to the
/// node until it checks them against the gate's own schema.
pub fn check_params(params: &BTreeMap<String, String>) -> Result<(), String> {
    if params.len() > MAX_PARAMS {
        return Err(format!("{} params, more than {MAX_PARAMS}", params.len()));
    }
    for (k, v) in params {
        if let Err(e) = check_token(k, MAX_PARAM_KEY, &['_', '-', '.']) {
            return Err(format!("param key {k:?}: {e}"));
        }
        if v.len() > MAX_PARAM_VALUE {
            return Err(format!(
                "param {k}: value is {} bytes, more than {MAX_PARAM_VALUE}",
                v.len()
            ));
        }
        if v.chars().any(char::is_control) {
            return Err(format!("param {k}: value contains a control character"));
        }
    }
    Ok(())
}

/// Where a job is in its life. Terminal means `Done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Preparing,
    Building,
    Running,
    Collecting,
    Done,
}

impl JobState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done)
    }
}

/// What one node may be asked about benchmark jobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BenchReq {
    /// The node's capability report.
    NodeInfo,
    /// Queue a gate at a commit. Idempotent on `job_key`.
    Submit {
        job_key: JobKey,
        sha: Sha,
        gate: GateId,
        #[serde(default)]
        params: BTreeMap<String, String>,
        /// A checkpoint the gate's baseline knows; the default when absent.
        #[serde(default)]
        checkpoint: Option<String>,
        /// The box class the caller expects; refused if it is not the node's.
        #[serde(default)]
        hardware: Option<String>,
        /// Caller's run deadline in seconds, clamped by the node.
        #[serde(default)]
        max_run_s: Option<u32>,
        /// Free text for the node's log.
        #[serde(default)]
        note: Option<String>,
    },
    /// One job, or all recent ones.
    Status {
        #[serde(default)]
        job: Option<JobId>,
    },
    /// Stop a job. Idempotent.
    Cancel { job: JobId },
    /// Turn this connection into an event stream for `job`, replaying from
    /// `from_seq` (inclusive) and then tailing until `Done`.
    Attach { job: JobId, from_seq: u64 },
    /// The artifacts a finished job produced.
    ArtifactList { job: JobId },
    /// One chunk of one artifact.
    ArtifactChunk {
        job: JobId,
        name: String,
        offset: u64,
        /// Bytes wanted; capped at [`MAX_CHUNK`].
        len: u32,
    },
}

/// A job in a status listing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSummary {
    pub job: JobId,
    pub job_key: JobKey,
    pub sha: Sha,
    pub gate: GateId,
    pub state: JobState,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    /// Highest event seq journaled so far.
    pub seq_high: u64,
    #[serde(default)]
    pub outcome: Option<super::bench_event::Outcome>,
}

/// The answer to a [`BenchReq`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BenchRep {
    NodeInfo {
        info: Box<super::bench_node::BenchNodeInfo>,
    },
    /// The job exists (created now, or already, per `existing`).
    Accepted {
        job: JobId,
        existing: bool,
        state: JobState,
        /// Place in the queue, 0 when running or done.
        position: u32,
    },
    Status {
        jobs: Vec<JobSummary>,
    },
    /// The job's state after the cancel; a terminal job answers with the
    /// state it already had.
    Cancelled {
        job: JobId,
        state: JobState,
    },
    Artifacts {
        job: JobId,
        items: Vec<super::bench_event::ArtifactMeta>,
    },
    Chunk {
        job: JobId,
        name: String,
        offset: u64,
        data_b64: String,
        eof: bool,
    },
    Refused {
        by: NodeId,
        refusal: BenchRefusal,
    },
}

/// Why a node said no. Every variant tells the operator what to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum BenchRefusal {
    /// The sender is pinned but has no `bench` right here.
    NotGranted {
        command: String,
    },
    /// The node has no `bench.yaml`, or it is unusable.
    NotConfigured {
        what: String,
        fix: String,
    },
    /// Something exclusive is already using the box.
    Busy {
        what: String,
        retry_after_s: u32,
    },
    MemoryPressure {
        available_frac: f64,
        required_frac: f64,
    },
    DiskLow {
        free_bytes: u64,
        required_bytes: u64,
    },
    QueueFull {
        depth: u32,
        retry_after_s: u32,
    },
    /// Same key, different request.
    KeyConflict {
        job: JobId,
    },
    /// The commit is not reachable from the configured remote.
    ShaNotAllowed {
        reason: String,
    },
    UnknownGate {
        gate: String,
        known: Vec<String>,
    },
    BadParams {
        errors: Vec<String>,
    },
    UnknownJob {
        job: JobId,
    },
    NoSuchArtifact {
        name: String,
    },
    RateLimited {
        retry_after_s: u32,
    },
    /// The peer speaks a protocol below what bench needs.
    Unsupported {
        version_max: u32,
    },
}

impl fmt::Display for BenchRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGranted { command } => write!(
                f,
                "this machine is paired but not granted bench here; on the node run: {command}"
            ),
            Self::NotConfigured { what, fix } => {
                write!(f, "bench is not configured ({what}); {fix}")
            }
            Self::Busy {
                what,
                retry_after_s,
            } => {
                write!(f, "the box is busy ({what}); retry after {retry_after_s} s")
            }
            Self::MemoryPressure {
                available_frac,
                required_frac,
            } => write!(
                f,
                "only {:.0} % of host memory is available; a gate needs {:.0} %",
                available_frac * 100.0,
                required_frac * 100.0
            ),
            Self::DiskLow {
                free_bytes,
                required_bytes,
            } => write!(
                f,
                "{} MiB free on the bench cache, {} MiB required",
                free_bytes >> 20,
                required_bytes >> 20
            ),
            Self::QueueFull {
                depth,
                retry_after_s,
            } => {
                write!(
                    f,
                    "the queue holds {depth} job(s); retry after {retry_after_s} s"
                )
            }
            Self::KeyConflict { job } => write!(
                f,
                "job key already names {job} with a different sha/gate/params; pick a new key"
            ),
            Self::ShaNotAllowed { reason } => write!(f, "commit refused: {reason}"),
            Self::UnknownGate { gate, known } => {
                write!(
                    f,
                    "unknown gate {gate}; this build knows: {}",
                    known.join(", ")
                )
            }
            Self::BadParams { errors } => write!(f, "bad params: {}", errors.join("; ")),
            Self::UnknownJob { job } => write!(f, "no such job {job}"),
            Self::NoSuchArtifact { name } => write!(f, "no such artifact {name}"),
            Self::RateLimited { retry_after_s } => {
                write!(f, "rate limited; retry after {retry_after_s} s")
            }
            Self::Unsupported { version_max } => write!(
                f,
                "the node speaks peer protocol {version_max}, and bench needs 3; upgrade it"
            ),
        }
    }
}
