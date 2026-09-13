// SPDX-License-Identifier: AGPL-3.0-only

//! How `atlasctl bench` fails: one class per exit code, one JSON shape.
//!
//! A script driving this command reads the exit code first and the
//! `ErrorObj` second; a human reads the message. Both come from the same
//! value, so they cannot disagree.

use atlasctl_agent::peer::bench::{DialError, UnsupportedPeer};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench_event::Outcome;
use atlasctl_protocol::msg::{BenchRefusal, BenchRep};
use serde::Serialize;
use std::fmt;

/// The exit code of a failed `bench` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Code {
    /// Bad arguments, or a local I/O failure.
    Usage = 1,
    /// The address did not answer, or the link broke before the hello.
    Unreachable = 2,
    /// The handshake failed: not paired on one side or the other.
    NotPaired = 3,
    /// The node answered with a typed refusal.
    Refused = 4,
    /// The job ended without a pass: failed, timed out, orphaned, or a
    /// completed run whose verdict was not `Pass`.
    JobFailed = 5,
    /// The job was cancelled.
    Cancelled = 6,
    /// The attached stream was lost and could not be re-established.
    StreamLost = 7,
    /// The node speaks a peer protocol below bench.
    Unsupported = 8,
}

impl Code {
    #[must_use]
    pub fn exit(self) -> std::process::ExitCode {
        std::process::ExitCode::from(self as u8)
    }
}

/// The machine-readable error.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ErrorObj {
    /// `unreachable`, `not_paired`, `not_granted`, `unsupported_version`,
    /// `refused:<code>`, `job_failed`, `job_cancelled`, `stream_lost`,
    /// `bad_args`, `io`.
    pub code: String,
    pub message: String,
    /// The address as given, when the failure names a node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The node's fingerprint, when it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// What to change, when the node or this command knows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Whether the same command may succeed later with nothing changed.
    pub retryable: bool,
}

/// A failure with its exit code.
#[derive(Debug)]
pub struct BenchError {
    pub exit: Code,
    /// Boxed so a `Result<_, BenchError>` stays pointer-sized on the Ok path.
    pub obj: Box<ErrorObj>,
    /// Render the error as JSON on stdout (set at the command boundary from
    /// the subcommand's `--json`).
    pub json: bool,
    /// Already on stdout as part of the command's own document; `main` must
    /// only set the exit code.
    pub reported: bool,
}

impl fmt::Display for BenchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.obj.node {
            Some(n) => write!(f, "{n}: {}", self.obj.message)?,
            None => write!(f, "{}", self.obj.message)?,
        }
        if let Some(fix) = &self.obj.fix {
            write!(f, "\n  fix: {fix}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BenchError {}

impl BenchError {
    fn new(exit: Code, code: &str, message: String) -> Self {
        Self {
            exit,
            obj: Box::new(ErrorObj {
                code: code.to_owned(),
                message,
                node: None,
                node_id: None,
                fix: None,
                retryable: false,
            }),
            json: false,
            reported: false,
        }
    }

    #[must_use]
    pub fn at(mut self, node: &str) -> Self {
        self.obj.node = Some(node.to_owned());
        self
    }

    #[must_use]
    pub fn by(mut self, id: NodeId) -> Self {
        self.obj.node_id = Some(id.to_string());
        self
    }

    #[must_use]
    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.obj.fix = Some(fix.into());
        self
    }

    #[must_use]
    pub fn retryable(mut self) -> Self {
        self.obj.retryable = true;
        self
    }

    pub fn bad_args(message: impl Into<String>) -> Self {
        Self::new(Code::Usage, "bad_args", message.into())
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(Code::Usage, "io", message.into())
    }

    pub fn unreachable(message: impl Into<String>) -> Self {
        Self::new(Code::Unreachable, "unreachable", message.into()).retryable()
    }

    pub fn not_paired(message: impl Into<String>) -> Self {
        Self::new(Code::NotPaired, "not_paired", message.into()).fix(
            "pair the two machines: `atlasctl peer list` on both shows who trusts whom; \
             the node must also `peer grant-bench <this machine>`",
        )
    }

    pub fn stream_lost(message: impl Into<String>) -> Self {
        Self::new(Code::StreamLost, "stream_lost", message.into()).retryable()
    }

    pub fn unsupported(e: &UnsupportedPeer) -> Self {
        Self::new(Code::Unsupported, "unsupported_version", e.to_string())
            .by(e.peer)
            .fix("upgrade atlasctl on the node")
    }

    /// A node's typed refusal, with its own code under `refused:`.
    pub fn refused(by: NodeId, refusal: &BenchRefusal) -> Self {
        let code = refusal_code(refusal);
        let (exit, code_str) = if code == "not_granted" {
            (Code::NotPaired, "not_granted".to_owned())
        } else {
            (Code::Refused, format!("refused:{code}"))
        };
        let mut e = Self::new(exit, &code_str, refusal.to_string()).by(by);
        e.obj.retryable = matches!(
            refusal,
            BenchRefusal::Busy { .. }
                | BenchRefusal::QueueFull { .. }
                | BenchRefusal::RateLimited { .. }
                | BenchRefusal::MemoryPressure { .. }
        );
        e.obj.fix = refusal_fix(refusal);
        e
    }

    /// How a job that did not pass is reported.
    pub fn outcome(by: NodeId, outcome: &Outcome) -> Self {
        let (exit, code) = match outcome {
            Outcome::Cancelled { .. } => (Code::Cancelled, "job_cancelled"),
            _ => (Code::JobFailed, "job_failed"),
        };
        Self::new(exit, code, outcome_text(outcome)).by(by)
    }

    /// Classify an error from the peer channel.
    pub fn from_link(node: &str, e: anyhow::Error) -> Self {
        if let Some(d) = e.downcast_ref::<DialError>() {
            let out = if d.not_paired() {
                Self::not_paired(d.to_string())
            } else if d.transient() {
                Self::unreachable(d.to_string())
            } else {
                // A handshake that broke without our verifier speaking is the
                // other side refusing us, or a wire fault; the fix is the same.
                Self::not_paired(format!(
                    "{d} — the node may not have this machine pinned, or the link broke mid-handshake"
                ))
            };
            return out.at(node);
        }
        if let Some(u) = e.downcast_ref::<UnsupportedPeer>() {
            return Self::unsupported(u).at(node);
        }
        Self::unreachable(format!("{e:#}")).at(node)
    }

    /// The reply was not the variant the request expects.
    pub fn unexpected(node: &str, rep: &BenchRep) -> Self {
        match rep {
            BenchRep::Refused { by, refusal } => Self::refused(*by, refusal).at(node),
            other => Self::unreachable(format!("unexpected reply {other:?}")).at(node),
        }
    }
}

fn refusal_code(r: &BenchRefusal) -> &'static str {
    match r {
        BenchRefusal::NotGranted { .. } => "not_granted",
        BenchRefusal::NotConfigured { .. } => "not_configured",
        BenchRefusal::Busy { .. } => "busy",
        BenchRefusal::MemoryPressure { .. } => "memory_pressure",
        BenchRefusal::DiskLow { .. } => "disk_low",
        BenchRefusal::QueueFull { .. } => "queue_full",
        BenchRefusal::KeyConflict { .. } => "key_conflict",
        BenchRefusal::ShaNotAllowed { .. } => "sha_not_allowed",
        BenchRefusal::UnknownGate { .. } => "unknown_gate",
        BenchRefusal::BadParams { .. } => "bad_params",
        BenchRefusal::UnknownJob { .. } => "unknown_job",
        BenchRefusal::NoSuchArtifact { .. } => "no_such_artifact",
        BenchRefusal::RateLimited { .. } => "rate_limited",
        BenchRefusal::Unsupported { .. } => "unsupported",
    }
}

fn refusal_fix(r: &BenchRefusal) -> Option<String> {
    Some(match r {
        BenchRefusal::NotGranted { command } => format!("on the node, run: {command}"),
        BenchRefusal::NotConfigured { fix, .. } => fix.clone(),
        BenchRefusal::Busy { retry_after_s, .. }
        | BenchRefusal::QueueFull { retry_after_s, .. }
        | BenchRefusal::RateLimited { retry_after_s } => {
            format!("retry after {retry_after_s} s")
        }
        BenchRefusal::ShaNotAllowed { .. } => {
            "push the commit to the node's trusted remote first".to_owned()
        }
        BenchRefusal::UnknownGate { known, .. } => format!("one of: {}", known.join(", ")),
        BenchRefusal::KeyConflict { .. } => "pass a fresh --job-key".to_owned(),
        BenchRefusal::Unsupported { .. } => "upgrade atlasctl on the node".to_owned(),
        _ => return None,
    })
}

/// One line for an outcome, for the error message and the text renderer.
#[must_use]
pub fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Completed {
            exit_code,
            verdict,
            record,
            ..
        } => {
            let v = verdict.as_ref().map_or_else(
                || "no verdict".to_owned(),
                |v| format!("{:?}: {}", v.kind, v.text),
            );
            match record {
                Some(r) => format!("exit {exit_code}, {v}, record {r}"),
                None => format!("exit {exit_code}, {v}, no record"),
            }
        }
        Outcome::Failed { stage, reason } => format!("failed while {stage:?}: {reason}"),
        Outcome::TimedOut { stage, after_s } => {
            format!("timed out while {stage:?} after {after_s} s")
        }
        Outcome::Cancelled { by } => format!("cancelled by {}", by.short()),
        Outcome::Orphaned { reason } => format!("orphaned: {reason}"),
    }
}

#[cfg(test)]
#[path = "exit_tests.rs"]
mod exit_tests;
