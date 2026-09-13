// SPDX-License-Identifier: AGPL-3.0-only

//! The human renderings. Pure: strings in, strings out, so every line an
//! operator reads has a test that pins it.

use super::exit::outcome_text;
use atlasctl_protocol::msg::bench::JobSummary;
use atlasctl_protocol::msg::bench_event::{EventKind, LogStream};
use atlasctl_protocol::msg::{BenchEvent, BenchNodeInfo};

fn gib(bytes: f64) -> String {
    format!("{:.1} GiB", bytes / (1024.0 * 1024.0 * 1024.0))
}

/// One node's report, as a block.
#[must_use]
pub fn node_block(given: &str, info: &BenchNodeInfo) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "{given}  {} ({})  atlasctl {}  bench {}\n",
        info.name.as_str(),
        info.node.short(),
        info.agent_version,
        if info.bench_enabled {
            "on".to_owned()
        } else {
            format!(
                "OFF — {}",
                info.disabled_reason.as_deref().unwrap_or("no reason given")
            )
        }
    ));
    match &info.gpu {
        Some(g) => {
            s.push_str(&format!(
                "  gpu      {}×{}  driver {}  cuda {}  clock {}  temp {}  mem {}\n",
                g.count,
                g.name,
                g.driver_version,
                if g.cuda_version.is_empty() {
                    "?"
                } else {
                    &g.cuda_version
                },
                g.sm_clock_mhz
                    .value()
                    .map_or("n/a".to_owned(), |v| format!("{v:.0} MHz")),
                g.temperature_c
                    .value()
                    .map_or("n/a".to_owned(), |v| format!("{v:.0} °C")),
                g.memory_total_bytes.value().map_or("n/a".to_owned(), gib),
            ));
        }
        None => s.push_str("  gpu      none reported\n"),
    }
    if let Some(t) = &info.thermal {
        s.push_str(&format!(
            "  thermal  chassis {}  throttle {}  clock max {}  mem {}\n",
            t.chassis_temps_c
                .iter()
                .cloned()
                .fold(None::<f64>, |m, x| Some(m.map_or(x, |m| m.max(x))))
                .map_or("n/a".to_owned(), |c| format!("{c:.0} °C")),
            match t.throttle_thermal {
                Some(true) => "ACTIVE",
                Some(false) => "none",
                None => "n/a",
            },
            t.sm_clock_max_mhz
                .map_or("n/a".to_owned(), |c| format!("{c:.0} MHz")),
            t.mem_total_kb
                .map_or("n/a".to_owned(), |kb| gib(kb as f64 * 1024.0)),
        ));
    }
    if let Some(class) = &info.hardware_class {
        s.push_str(&format!("  class    {class}\n"));
    }
    if let Some(repo) = &info.atlas_repo {
        s.push_str(&format!(
            "  repo     {}  {}={}  head {}\n",
            repo.path,
            repo.remote_name,
            repo.remote_url,
            repo.head_sha
                .as_ref()
                .map_or("?".to_owned(), |h| h.short().to_owned()),
        ));
    }
    if let Some(fp) = &info.signer_fp {
        s.push_str(&format!("  signer   {fp}\n"));
    }
    s.push_str(&format!(
        "  built    {}\n",
        if info.built_shas.is_empty() {
            "nothing cached".to_owned()
        } else {
            info.built_shas
                .iter()
                .map(|b| b.sha.short().to_owned())
                .collect::<Vec<_>>()
                .join(" ")
        }
    ));
    s.push_str(&format!(
        "  queue    {}{}  {} queued of {}  max run {} s\n",
        if info.busy { "BUSY" } else { "idle" },
        info.busy_reason
            .as_deref()
            .map_or(String::new(), |r| format!(" ({r})")),
        info.queued,
        info.queue_depth,
        info.max_run_s
    ));
    if let Some(j) = &info.running_job {
        s.push_str(&format!(
            "  running  {} {} @ {} ({:?})\n",
            j.job,
            j.gate,
            j.sha.short(),
            j.state
        ));
    }
    s.push_str(&format!(
        "  host     mem free {}  disk free {}\n",
        info.host_free_fraction
            .value()
            .map_or("n/a".to_owned(), |v| format!("{:.0} %", v * 100.0)),
        info.disk_free_bytes.value().map_or("n/a".to_owned(), gib),
    ));
    for a in &info.alerts {
        s.push_str(&format!("  ALERT    {:?}: {}\n", a.kind, a.detail));
    }
    s
}

/// The jobs table.
#[must_use]
pub fn status_table(jobs: &[JobSummary]) -> String {
    if jobs.is_empty() {
        return "no jobs\n".to_owned();
    }
    let mut s = format!(
        "{:<22}  {:<10}  {:<26}  {:<10}  {:>5}  OUTCOME\n",
        "JOB", "STATE", "GATE", "SHA", "SEQ"
    );
    for j in jobs {
        s.push_str(&format!(
            "{:<22}  {:<10}  {:<26}  {:<10}  {:>5}  {}\n",
            j.job.to_string(),
            format!("{:?}", j.state).to_ascii_lowercase(),
            j.gate.to_string(),
            j.sha.short(),
            j.seq_high,
            j.outcome.as_ref().map_or(String::new(), outcome_text)
        ));
    }
    s
}

/// One event, one line (log batches: one line per log line), or nothing
/// for events a human does not need.
#[must_use]
pub fn event_lines(ev: &BenchEvent) -> Vec<String> {
    let t = ev.at_ms / 1000;
    let stamp = format!("{:02}:{:02}:{:02}", (t / 3600) % 24, (t / 60) % 60, t % 60);
    let one = |body: String| vec![format!("[{stamp} #{}] {body}", ev.seq)];
    match &ev.kind {
        EventKind::Queued { position } => one(format!("queued at position {position}")),
        EventKind::Preparing { sha, fetched } => one(format!(
            "preparing {}{}",
            sha.short(),
            if *fetched { " (fetched)" } else { "" }
        )),
        EventKind::Build { cached, reason } => one(if *cached {
            format!("build: cached ({reason})")
        } else {
            format!("build: {reason}")
        }),
        EventKind::Built {
            binary_sha256,
            cached,
            secs,
        } => one(format!(
            "built {} in {secs}s{}",
            &binary_sha256[..binary_sha256.len().min(12)],
            if *cached { " (cache hit)" } else { "" }
        )),
        EventKind::Running { pid, argv } => one(format!("running pid {pid}: {}", argv.join(" "))),
        EventKind::Progress { phase, detail } => one(if detail.is_empty() {
            phase.clone()
        } else {
            format!("{phase}  {detail}")
        }),
        EventKind::Log { stream, lines } => {
            let tag = match stream {
                LogStream::Build => "build",
                LogStream::Run => "run",
            };
            lines.iter().map(|l| format!("  {tag}| {l}")).collect()
        }
        EventKind::LogTruncated { dropped_bytes } => {
            one(format!("… {dropped_bytes} bytes of log not forwarded"))
        }
        EventKind::Verdict { verdict } => one(format!("{:?}: {}", verdict.kind, verdict.text)),
        EventKind::Artifact { meta } => one(format!(
            "artifact {} ({} bytes)",
            meta.relative_path, meta.bytes
        )),
        EventKind::Done { outcome } => one(format!("done — {}", outcome_text(outcome))),
        EventKind::Heartbeat { .. } => vec![],
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
