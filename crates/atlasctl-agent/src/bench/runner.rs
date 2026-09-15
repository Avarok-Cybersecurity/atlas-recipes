// SPDX-License-Identifier: AGPL-3.0-only

//! The single worker: one job at a time, recovery after a restart, and
//! retention.
//!
//! One job at a time is the point, not a limitation — a gate serves a model
//! that takes most of unified memory and measures speed. Before each start
//! the box is re-checked for anything exclusive (another `spark`, a GPU
//! compute app, memory, disk), because those change between submit and
//! start.

use super::child::same_process;
use super::exclusive;
use super::host::BenchHost;
use super::job::JobRecord;
use super::lease;
use super::machine::{self, Ctx};
use super::ports_std::StdPorts;
use anyhow::Result;
use atlasctl_protocol::msg::bench::JobState;
use atlasctl_protocol::msg::bench_event::EventKind;
use std::sync::Arc;
use std::time::Duration;

/// How long the worker waits between looks at the queue when nothing woke
/// it, and how long a busy box is left before the next probe.
pub const IDLE_POLL: Duration = Duration::from_secs(5);
pub const BUSY_RETRY: Duration = Duration::from_secs(30);

/// Run the worker until the runtime shuts down.
pub async fn run(host: Arc<BenchHost>) {
    let ports = Arc::new(StdPorts::new(host.cfg.clone()));
    if let Err(e) = recover(&host, &ports).await {
        tracing_warn(&format!("bench: recovery failed: {e:#}"));
    }
    let mut idle_since: Option<std::time::Instant> = None;
    loop {
        match next_job(&host) {
            Some(job) => {
                idle_since = None;
                let readings = exclusive::probe(
                    &host.cfg.cache_dir,
                    host.running().map(|j| j.to_string()),
                    &host.cfg.atlas_home,
                );
                let busy = exclusive::judge(
                    &readings,
                    host.cfg.min_free_fraction,
                    host.cfg.min_free_disk_bytes,
                );
                if !busy.is_empty() {
                    tracing_warn(&format!(
                        "bench: {} waits — {}",
                        job.id,
                        busy.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    ));
                    tokio::time::sleep(BUSY_RETRY).await;
                    continue;
                }
                execute(&host, &ports, job).await;
                if let Err(e) = retain(&host) {
                    tracing_warn(&format!("bench: retention failed: {e:#}"));
                }
            }
            None => {
                release_idle_lease(&host, &mut idle_since);
                tokio::select! {
                    () = host.wake.notified() => {}
                    () = tokio::time::sleep(IDLE_POLL) => {}
                }
            }
        }
    }
}

/// Stop the server the last job left leased once the queue has been empty
/// for `serve_release_after_s`: a model kept warm for a job that is not
/// coming is a box nobody else can use.
fn release_idle_lease(host: &BenchHost, idle_since: &mut Option<std::time::Instant>) {
    let Some(l) = lease::ours(&host.cfg.atlas_home) else {
        *idle_since = None;
        return;
    };
    let since = *idle_since.get_or_insert_with(std::time::Instant::now);
    if since.elapsed() < Duration::from_secs(u64::from(host.cfg.serve_release_after_s)) {
        return;
    }
    tracing_info(&format!(
        "bench: releasing the leased server (pid {}, port {}, {}) after {} s idle",
        l.pid,
        l.port,
        l.model,
        since.elapsed().as_secs()
    ));
    lease::release(
        &host.cfg.atlas_home,
        &l,
        Duration::from_secs(u64::from(host.cfg.cancel_grace_s)),
    );
    *idle_since = None;
}

fn next_job(host: &BenchHost) -> Option<JobRecord> {
    host.queued().ok()?.into_iter().next()
}

/// Run one job on a blocking thread, with its cancel flag registered so a
/// `Cancel` frame reaches it.
async fn execute(host: &Arc<BenchHost>, ports: &Arc<StdPorts>, job: JobRecord) {
    let id = job.id.clone();
    let cancel = host.register_cancel(&id);
    host.set_running(Some(id.clone()));
    let journal = match host.journal(&id) {
        Ok(j) => j,
        Err(e) => {
            tracing_warn(&format!("bench: {id}: cannot open its journal: {e:#}"));
            host.set_running(None);
            host.forget_cancel(&id);
            return;
        }
    };
    let h = host.clone();
    let p = ports.clone();
    let result = tokio::task::spawn_blocking(move || {
        let ctx = Ctx {
            ports: p.as_ref(),
            store: &h.store,
            journal: &journal,
            local: h.local,
            cancel,
            hardware: &h.cfg.hardware,
            child_env: h.cfg.child_env(),
            build_timeout: Duration::from_secs(u64::from(h.cfg.build_timeout_s)),
            stall_timeout: Duration::from_secs(u64::from(h.cfg.stall_timeout_s)),
            allow_unpublished: h.cfg.allow_unpublished_shas,
            serve_reuse: h.cfg.serve_reuse,
        };
        machine::run(&ctx, job)
    })
    .await;
    match result {
        Ok(Ok(outcome)) => tracing_info(&format!("bench: {id} → {outcome:?}")),
        Ok(Err(e)) => tracing_warn(&format!("bench: {id}: could not record its outcome: {e:#}")),
        Err(e) => tracing_warn(&format!("bench: {id}: worker panicked: {e}")),
    }
    host.set_running(None);
    host.forget_cancel(&id);
}

/// After a restart: a job whose child is still the same process is waited
/// for and collected; any other non-terminal job is orphaned, and queued
/// jobs stay queued.
async fn recover(host: &Arc<BenchHost>, ports: &Arc<StdPorts>) -> Result<()> {
    for job in host.store.list()? {
        match job.state {
            JobState::Done | JobState::Queued => continue,
            JobState::Running => {
                let alive = matches!((job.child_pid, job.child_start_ticks), (Some(pid), Some(t)) if same_process(pid, t));
                if alive {
                    tracing_info(&format!(
                        "bench: {} is still running (pid {:?}); resuming",
                        job.id, job.child_pid
                    ));
                    // Resume by re-entering the run phase's wait through the
                    // machine: simplest correct path is to let the machine's
                    // wait see the live pid via /proc, which `StdPorts::wait`
                    // does when it has no handle.
                    let id = job.id.clone();
                    let cancel = host.register_cancel(&id);
                    host.set_running(Some(id.clone()));
                    let journal = host.journal(&id)?;
                    let h = host.clone();
                    let p = ports.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let ctx = Ctx {
                            ports: p.as_ref(),
                            store: &h.store,
                            journal: &journal,
                            local: h.local,
                            cancel,
                            hardware: &h.cfg.hardware,
                            child_env: h.cfg.child_env(),
                            build_timeout: Duration::from_secs(u64::from(h.cfg.build_timeout_s)),
                            stall_timeout: Duration::from_secs(u64::from(h.cfg.stall_timeout_s)),
                            allow_unpublished: h.cfg.allow_unpublished_shas,
                            serve_reuse: h.cfg.serve_reuse,
                        };
                        machine::resume_running(&ctx, job)
                    })
                    .await;
                    host.set_running(None);
                    host.forget_cancel(&id);
                } else {
                    orphan(
                        host,
                        ports,
                        job,
                        "the agent restarted and the child was gone",
                    )
                    .await?;
                }
            }
            JobState::Preparing | JobState::Building | JobState::Collecting => {
                orphan(host, ports, job, "the agent restarted mid-job").await?;
            }
        }
    }
    Ok(())
}

async fn orphan(
    host: &Arc<BenchHost>,
    ports: &Arc<StdPorts>,
    job: JobRecord,
    why: &str,
) -> Result<()> {
    let journal = host.journal(&job.id)?;
    let ctx = Ctx {
        ports: ports.as_ref(),
        store: &host.store,
        journal: &journal,
        local: host.local,
        cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        hardware: &host.cfg.hardware,
        child_env: vec![],
        build_timeout: Duration::ZERO,
        stall_timeout: Duration::ZERO,
        allow_unpublished: false,
        serve_reuse: false,
    };
    machine::orphan(&ctx, job, why)?;
    Ok(())
}

/// Prune terminal jobs beyond the retention limits, oldest first, and keep
/// only the newest `keep_builds` build directories (never the running job's).
pub fn retain(host: &BenchHost) -> Result<()> {
    let now = super::job::now_s();
    let mut done: Vec<JobRecord> = host
        .store
        .list()?
        .into_iter()
        .filter(|j| j.state.is_terminal())
        .collect();
    done.sort_by_key(|j| std::cmp::Reverse(j.updated_at_s));
    for (i, j) in done.iter().enumerate() {
        let too_many = i >= host.cfg.retain_jobs as usize;
        let too_old = now.saturating_sub(j.updated_at_s) > u64::from(host.cfg.retain_days) * 86_400;
        if too_many || too_old {
            let _ = host.store.remove(j);
        }
    }
    let builds = host.cfg.builds_dir();
    let running_sha = host
        .running()
        .and_then(|id| host.store.load(&id).ok())
        .map(|j| j.spec.sha.as_str().to_string());
    let mut entries: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&builds)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let t = std::fs::metadata(p.join("spark"))
                .and_then(|m| m.modified())
                .ok()?;
            Some((t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(), p))
        })
        .collect();
    entries.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, p) in entries.into_iter().skip(host.cfg.keep_builds as usize) {
        if running_sha.as_deref() == p.file_name().and_then(|n| n.to_str()) {
            continue;
        }
        let _ = std::fs::remove_dir_all(&p);
        let _ = std::fs::remove_dir_all(
            host.cfg
                .worktrees_dir()
                .join(p.file_name().unwrap_or_default()),
        );
    }
    let _ = EventKind::Queued { position: 0 };
    Ok(())
}

fn tracing_warn(msg: &str) {
    eprintln!("{msg}");
}
fn tracing_info(msg: &str) {
    eprintln!("{msg}");
}
