// SPDX-License-Identifier: AGPL-3.0-only

//! One job's life, as a procedure over ports.
//!
//! Queued → Preparing (the commit is here, and published) → Building (a
//! cached binary for this exact sha, or a fresh build) → Running (the gate
//! as a child in its own process group, output to a file) → Collecting (the
//! records it wrote) → Done. Every step writes its transition to the job
//! record and its events to the journal before the next begins, so a
//! restart finds the truth on disk.
//!
//! Nothing here touches git, cargo or a process directly: [`Ports`] does,
//! and the tests drive this with a scripted one.

use super::job::JobRecord;
use anyhow::Result;
use atlasctl_protocol::msg::bench::JobState;
use atlasctl_protocol::msg::bench_event::{EventKind, Outcome};
use std::sync::atomic::Ordering;
use std::time::Duration;

pub use super::ports::{
    BuildResult, CacheMiss, CachedBinary, ChildHandle, Ctx, Ports, RunEnd, RunPlan,
};

mod resume;
pub use resume::{orphan, resume_running};

fn transition(ctx: &Ctx, job: &mut JobRecord, state: JobState) -> Result<()> {
    job.state = state;
    job.updated_at_s = ctx.ports.now_ms() / 1000;
    job.seq_high = ctx.journal.seq_high();
    ctx.store.save(job)
}

fn emit(ctx: &Ctx, job: &mut JobRecord, kind: EventKind) -> Result<()> {
    ctx.journal.append(kind, ctx.ports.now_ms())?;
    job.seq_high = ctx.journal.seq_high();
    Ok(())
}

fn finish(ctx: &Ctx, job: &mut JobRecord, outcome: Outcome) -> Result<Outcome> {
    job.child_pid = None;
    job.child_start_ticks = None;
    job.outcome = Some(outcome.clone());
    emit(
        ctx,
        job,
        EventKind::Done {
            outcome: outcome.clone(),
        },
    )?;
    transition(ctx, job, JobState::Done)?;
    Ok(outcome)
}

fn failed(stage: JobState, reason: impl Into<String>) -> Outcome {
    Outcome::Failed {
        stage,
        reason: reason.into(),
    }
}

/// Run a queued job to its end. Returns the outcome it recorded.
///
/// # Errors
/// Only when the job store or journal cannot be written — the job's own
/// failures are outcomes, not errors.
pub fn run(ctx: &Ctx, mut job: JobRecord) -> Result<Outcome> {
    let cancelled = || ctx.cancel.load(Ordering::SeqCst);
    let by = ctx.local;
    let sha = job.spec.sha.clone();
    let gate = job.spec.gate.clone();

    // ── Preparing ──
    transition(ctx, &mut job, JobState::Preparing)?;
    let fetched = match ctx.ports.has_commit(&sha) {
        Ok(true) => false,
        Ok(false) => {
            if let Err(e) = ctx.ports.fetch() {
                return finish(
                    ctx,
                    &mut job,
                    failed(JobState::Preparing, format!("fetch failed: {e:#}")),
                );
            }
            match ctx.ports.has_commit(&sha) {
                Ok(true) => true,
                Ok(false) => {
                    return finish(
                        ctx,
                        &mut job,
                        failed(
                            JobState::Preparing,
                            format!("commit {sha} is not on the remote"),
                        ),
                    );
                }
                Err(e) => {
                    return finish(ctx, &mut job, failed(JobState::Preparing, format!("{e:#}")));
                }
            }
        }
        Err(e) => return finish(ctx, &mut job, failed(JobState::Preparing, format!("{e:#}"))),
    };
    if !ctx.allow_unpublished {
        match ctx.ports.published(&sha) {
            Ok(true) => {}
            Ok(false) => {
                return finish(
                    ctx,
                    &mut job,
                    failed(
                        JobState::Preparing,
                        format!(
                            "commit {sha} is not reachable from the trusted remote; push it first (or set allow_unpublished_shas on this node)"
                        ),
                    ),
                );
            }
            Err(e) => return finish(ctx, &mut job, failed(JobState::Preparing, format!("{e:#}"))),
        }
    }
    emit(
        ctx,
        &mut job,
        EventKind::Preparing {
            sha: sha.clone(),
            fetched,
        },
    )?;
    let worktree = match ctx.ports.worktree(&sha) {
        Ok(w) => w,
        Err(e) => {
            return finish(
                ctx,
                &mut job,
                failed(JobState::Preparing, format!("worktree: {e:#}")),
            );
        }
    };
    if cancelled() {
        return finish(ctx, &mut job, Outcome::Cancelled { by });
    }

    // ── Building ──
    transition(ctx, &mut job, JobState::Building)?;
    let binary = match ctx.ports.cached_binary(&sha) {
        Ok(Ok(b)) => {
            emit(
                ctx,
                &mut job,
                EventKind::Build {
                    cached: true,
                    reason: format!("cache hit for {}", sha.short()),
                },
            )?;
            emit(
                ctx,
                &mut job,
                EventKind::Built {
                    binary_sha256: b.sha256.clone(),
                    cached: true,
                    secs: 0,
                },
            )?;
            b
        }
        Ok(Err(miss)) => {
            let reason = match miss {
                CacheMiss::NoBuild => "no build for this commit".to_string(),
                CacheMiss::NoProvenance => "a build directory with no provenance".to_string(),
                CacheMiss::WrongSha(s) => format!("provenance names {s}, not {}", sha.short()),
                CacheMiss::HashMismatch => "binary hash does not match its provenance".to_string(),
            };
            emit(
                ctx,
                &mut job,
                EventKind::Build {
                    cached: false,
                    reason,
                },
            )?;
            match ctx.ports.build(&sha, &worktree, &ctx.cancel) {
                Ok(r) => {
                    emit(
                        ctx,
                        &mut job,
                        EventKind::Built {
                            binary_sha256: r.binary.sha256.clone(),
                            cached: false,
                            secs: r.secs,
                        },
                    )?;
                    r.binary
                }
                Err(e) if cancelled() => {
                    let _ = e;
                    return finish(ctx, &mut job, Outcome::Cancelled { by });
                }
                Err(e) => {
                    return finish(ctx, &mut job, failed(JobState::Building, format!("{e:#}")));
                }
            }
        }
        Err(e) => return finish(ctx, &mut job, failed(JobState::Building, format!("{e:#}"))),
    };
    job.binary_sha256 = Some(binary.sha256.clone());
    ctx.store.save(&job)?;
    if cancelled() {
        return finish(ctx, &mut job, Outcome::Cancelled { by });
    }
    if let Err(e) = ctx.ports.ensure_recipes(&binary.path) {
        return finish(
            ctx,
            &mut job,
            failed(JobState::Building, format!("recipes: {e:#}")),
        );
    }
    // The gate and its params, checked against THIS binary's own schema.
    match ctx.ports.known_gates(&binary.path) {
        Ok(known) if !known.iter().any(|k| k == gate.as_str()) => {
            return finish(
                ctx,
                &mut job,
                failed(
                    JobState::Building,
                    format!(
                        "gate {gate} is unknown to the binary at {}; it knows: {}",
                        sha.short(),
                        known.join(", ")
                    ),
                ),
            );
        }
        Ok(_) => {}
        Err(e) => return finish(ctx, &mut job, failed(JobState::Building, format!("{e:#}"))),
    }
    let keys: Vec<String> = job.spec.params.keys().cloned().collect();
    match ctx.ports.bad_params(&binary.path, gate.as_str(), &keys) {
        Ok(bad) if !bad.is_empty() => {
            return finish(
                ctx,
                &mut job,
                failed(
                    JobState::Building,
                    format!("params not accepted by {gate}: {}", bad.join(", ")),
                ),
            );
        }
        Ok(_) => {}
        Err(e) => return finish(ctx, &mut job, failed(JobState::Building, format!("{e:#}"))),
    }

    // ── Running ──
    let mut argv = vec![
        binary.path.display().to_string(),
        "benchmark".into(),
        "run".into(),
        gate.as_str().to_string(),
        "--pull-request-gate".into(),
        "--hardware".into(),
        ctx.hardware.to_string(),
        "--yes".into(),
    ];
    if ctx.serve_reuse {
        argv.push("--serve-reuse".into());
        argv.push("--serve-lease-owner".into());
        argv.push(std::process::id().to_string());
    }
    if let Some(c) = &job.spec.checkpoint {
        argv.push("--checkpoint".into());
        argv.push(c.clone());
    }
    for (k, v) in &job.spec.params {
        argv.push("--param".into());
        argv.push(format!("{k}={v}"));
    }
    let plan = RunPlan {
        argv,
        cwd: worktree.clone(),
        env: ctx.child_env.clone(),
        log_path: ctx.store.child_log_path(&job.id),
        stall_timeout: ctx.stall_timeout,
        max_run: Duration::from_secs(u64::from(job.spec.max_run_s)),
    };
    let started_s = ctx.ports.now_ms() / 1000;
    let child = match ctx.ports.spawn(&plan) {
        Ok(c) => c,
        Err(e) => {
            return finish(
                ctx,
                &mut job,
                failed(JobState::Running, format!("spawn: {e:#}")),
            );
        }
    };
    job.child_pid = Some(child.pid);
    job.child_start_ticks = Some(child.start_ticks);
    transition(ctx, &mut job, JobState::Running)?;
    emit(
        ctx,
        &mut job,
        EventKind::Running {
            pid: child.pid,
            argv: plan.argv.clone(),
        },
    )?;
    let journal = ctx.journal.clone();
    let now = || ctx.ports.now_ms();
    let end = ctx.ports.wait(&child, &plan, &ctx.cancel, &|k| {
        let _ = journal.append(k, now());
    });
    job.seq_high = ctx.journal.seq_high();
    let (exit_code, verdict) = match end {
        Ok(RunEnd::Exited { code, verdict }) => (code, verdict),
        Ok(RunEnd::Stalled { after_s }) | Ok(RunEnd::TimedOut { after_s }) => {
            return finish(
                ctx,
                &mut job,
                Outcome::TimedOut {
                    stage: JobState::Running,
                    after_s,
                },
            );
        }
        Ok(RunEnd::Cancelled) => return finish(ctx, &mut job, Outcome::Cancelled { by }),
        Err(e) => return finish(ctx, &mut job, failed(JobState::Running, format!("{e:#}"))),
    };
    if let Some(v) = &verdict {
        emit(ctx, &mut job, EventKind::Verdict { verdict: v.clone() })?;
    }

    // ── Collecting ──
    transition(ctx, &mut job, JobState::Collecting)?;
    let dest = ctx.store.artifacts_dir(&job.id);
    let artifacts = match ctx
        .ports
        .collect(&worktree, gate.as_str(), started_s, &dest)
    {
        Ok(a) => a,
        Err(e) => {
            return finish(
                ctx,
                &mut job,
                failed(JobState::Collecting, format!("{e:#}")),
            );
        }
    };
    for a in &artifacts {
        emit(ctx, &mut job, EventKind::Artifact { meta: a.clone() })?;
    }
    let record = artifacts
        .iter()
        .find(|a| a.kind == atlasctl_protocol::msg::bench_event::ArtifactKind::Record)
        .map(|a| a.relative_path.clone());
    let signature = artifacts
        .iter()
        .find(|a| a.kind == atlasctl_protocol::msg::bench_event::ArtifactKind::Signature)
        .map(|a| a.relative_path.clone());
    if exit_code == 0 && record.is_none() {
        return finish(
            ctx,
            &mut job,
            failed(
                JobState::Collecting,
                "the child exited 0 but wrote no gate record",
            ),
        );
    }
    finish(
        ctx,
        &mut job,
        Outcome::Completed {
            exit_code,
            verdict,
            record,
            signature,
        },
    )
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod machine_tests;
