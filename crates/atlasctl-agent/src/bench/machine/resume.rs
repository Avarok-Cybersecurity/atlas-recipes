// SPDX-License-Identifier: AGPL-3.0-only

//! The two entry points a restart needs: resume a child that outlived the
//! agent, or record that a job could not be recovered.

use super::{ChildHandle, Ctx, RunEnd, RunPlan, emit, failed, finish, transition};
use crate::bench::job::JobRecord;
use anyhow::{Context, Result};
use atlasctl_protocol::msg::bench::JobState;
use atlasctl_protocol::msg::bench_event::{EventKind, Outcome};
use std::time::Duration;

/// Resume a job whose child survived an agent restart: wait for the pid the
/// record names, then collect exactly as a fresh run would.
///
/// # Errors
/// Only when the job store or journal cannot be written.
pub fn resume_running(ctx: &Ctx, mut job: JobRecord) -> Result<Outcome> {
    let by = ctx.local;
    let (Some(pid), Some(start_ticks)) = (job.child_pid, job.child_start_ticks) else {
        return finish(
            ctx,
            &mut job,
            Outcome::Orphaned {
                reason: "no child recorded".into(),
            },
        );
    };
    let gate = job.spec.gate.clone();
    let worktree = match ctx.ports.worktree(&job.spec.sha) {
        Ok(w) => w,
        Err(e) => {
            return finish(
                ctx,
                &mut job,
                Outcome::Orphaned {
                    reason: format!("worktree: {e:#}"),
                },
            );
        }
    };
    let plan = RunPlan {
        argv: vec![],
        cwd: worktree.clone(),
        env: ctx.child_env.clone(),
        log_path: ctx.store.child_log_path(&job.id),
        stall_timeout: ctx.stall_timeout,
        max_run: Duration::from_secs(u64::from(job.spec.max_run_s)),
    };
    let child = ChildHandle { pid, start_ticks };
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
    transition(ctx, &mut job, JobState::Collecting)?;
    let dest = ctx.store.artifacts_dir(&job.id);
    let artifacts = match ctx
        .ports
        .collect(&worktree, gate.as_str(), &job.records_before, &dest)
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

/// Mark a job the agent could not recover after a restart.
///
/// # Errors
/// On I/O.
pub fn orphan(ctx: &Ctx, mut job: JobRecord, reason: &str) -> Result<()> {
    finish(
        ctx,
        &mut job,
        Outcome::Orphaned {
            reason: reason.into(),
        },
    )
    .context("recording an orphaned job")?;
    Ok(())
}
