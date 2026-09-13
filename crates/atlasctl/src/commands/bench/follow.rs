// SPDX-License-Identifier: AGPL-3.0-only

//! Following a job to its end across dropped links.
//!
//! The node journals every event with a monotonic `seq`, so a follower that
//! loses its connection re-attaches from `last_seq + 1` and misses nothing,
//! duplicates nothing. The only decision here is how long to keep trying:
//! a budget per outage, not per job, because a gate legitimately runs for
//! hours and a day-long budget must cover a blip at hour three as well as
//! at minute one.

use super::address::Target;
use super::exit::BenchError;
use super::session::Session;
use atlasctl_agent::peer::bench::AttachRefused;
use atlasctl_protocol::msg::bench::JobId;
use atlasctl_protocol::msg::bench_event::{ArtifactMeta, EventKind, Outcome, Verdict};
use atlasctl_protocol::msg::{BenchEvent, BenchRep, BenchReq};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// First pause before a re-attach; doubles up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// What a followed job left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Followed {
    pub outcome: Outcome,
    pub last_seq: u64,
    pub verdict: Option<Verdict>,
    pub artifacts: Vec<ArtifactMeta>,
}

/// The policy, separated from the sockets so it is testable.
#[derive(Debug, Clone, Copy)]
pub struct Reconnect {
    pub budget: Duration,
}

impl Reconnect {
    /// Whether to try again after a drop that began at `dropped_at`, and
    /// how long to wait first.
    #[must_use]
    pub fn next_attempt(self, dropped_at: Instant, now: Instant, attempt: u32) -> Option<Duration> {
        if self.budget.is_zero() || now.duration_since(dropped_at) >= self.budget {
            return None;
        }
        let pause = RETRY_MIN
            .saturating_mul(1u32 << attempt.min(4))
            .min(RETRY_MAX);
        Some(pause)
    }
}

/// Fold events into what the caller keeps.
#[derive(Debug, Default)]
pub struct Tally {
    pub verdict: Option<Verdict>,
    pub artifacts: Vec<ArtifactMeta>,
}

impl Tally {
    /// Note an event; returns the outcome when it is the last one.
    pub fn note(&mut self, ev: &BenchEvent) -> Option<Outcome> {
        match &ev.kind {
            EventKind::Verdict { verdict } => self.verdict = Some(verdict.clone()),
            EventKind::Artifact { meta } => self.artifacts.push(meta.clone()),
            EventKind::Done { outcome } => return Some(outcome.clone()),
            _ => {}
        }
        None
    }
}

/// Attach and follow until `Done`, re-attaching within `policy` on a drop.
///
/// `emit` sees every journaled event once, in order.
///
/// # Errors
/// `stream_lost` when the budget runs out; a refusal on re-attach (the job
/// is gone) is that refusal; any other link failure is classified.
pub fn follow(
    session: &Session,
    target: &Target,
    addr: SocketAddr,
    job: &JobId,
    from_seq: u64,
    policy: Reconnect,
    emit: &mut dyn FnMut(&BenchEvent),
) -> Result<Followed, BenchError> {
    let mut next_seq = from_seq.max(1);
    let mut tally = Tally::default();
    let mut outage: Option<(Instant, u32)> = None;
    loop {
        let mut attached = match session.attach(target, addr, job, next_seq) {
            Ok(a) => a,
            Err(e) => {
                retry_or(&mut outage, policy, e)?;
                continue;
            }
        };
        loop {
            match session.next_event(&mut attached) {
                Ok(Some(ev)) => {
                    outage = None;
                    next_seq = ev.seq + 1;
                    emit(&ev);
                    if let Some(outcome) = tally.note(&ev) {
                        return Ok(Followed {
                            outcome,
                            last_seq: ev.seq,
                            verdict: tally.verdict,
                            artifacts: tally.artifacts,
                        });
                    }
                }
                Ok(None) => {
                    // The node ended the stream on purpose: the job is
                    // terminal and nothing lies past `from_seq`. Its outcome
                    // is on the record, not in what we streamed.
                    return terminal_outcome(session, target, addr, job, &tally, next_seq - 1);
                }
                Err(e) => {
                    if let Some(r) = e.downcast_ref::<AttachRefused>() {
                        return Err(BenchError::refused(r.by, &r.refusal).at(&target.given));
                    }
                    let classified = BenchError::stream_lost(format!("{e:#}")).at(&target.given);
                    retry_or(&mut outage, policy, classified)?;
                    break;
                }
            }
        }
    }
}

/// The outcome of a job whose Done event lies before `from_seq`.
fn terminal_outcome(
    session: &Session,
    target: &Target,
    addr: SocketAddr,
    job: &JobId,
    tally: &Tally,
    last_seq: u64,
) -> Result<Followed, BenchError> {
    let (_, rep) = session.request(
        target,
        &[addr],
        &BenchReq::Status {
            job: Some(job.clone()),
        },
    )?;
    let BenchRep::Status { jobs } = rep else {
        return Err(BenchError::unexpected(&target.given, &rep));
    };
    match jobs
        .into_iter()
        .find(|j| &j.job == job)
        .and_then(|j| j.outcome)
    {
        Some(outcome) => Ok(Followed {
            outcome,
            last_seq,
            verdict: tally.verdict.clone(),
            artifacts: tally.artifacts.clone(),
        }),
        None => Err(BenchError::stream_lost(format!(
            "{} ended the stream for {job} (last seq {last_seq}) but reports no outcome",
            target.given
        ))
        .at(&target.given)),
    }
}

/// Sleep for the next attempt, or return `err` when the outage has used
/// its budget. Also final for anything that is not a link fault.
fn retry_or(
    outage: &mut Option<(Instant, u32)>,
    policy: Reconnect,
    err: BenchError,
) -> Result<(), BenchError> {
    if !err.obj.retryable {
        return Err(err);
    }
    let now = Instant::now();
    let (since, attempt) = outage.get_or_insert((now, 0));
    match policy.next_attempt(*since, now, *attempt) {
        Some(pause) => {
            eprintln!(
                "link lost ({}); re-attaching in {}s",
                err.obj.message,
                pause.as_secs()
            );
            *attempt += 1;
            std::thread::sleep(pause);
            Ok(())
        }
        None => Err(BenchError::stream_lost(format!(
            "{} (gave up after {}s of re-attaching)",
            err.obj.message,
            policy.budget.as_secs()
        ))
        .at(err.obj.node.as_deref().unwrap_or("?"))),
    }
}

#[cfg(test)]
#[path = "follow_tests.rs"]
mod follow_tests;
