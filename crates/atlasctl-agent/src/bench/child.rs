// SPDX-License-Identifier: AGPL-3.0-only

//! What the agent knows about the gate child it started: whether a pid is
//! still that process, how to signal its group, and what its output lines
//! mean. Pure functions over `/proc` and text; [`ports_std`](super::ports_std)
//! is the only caller that spawns.

use anyhow::Result;
use atlasctl_protocol::msg::bench::MAX_LOG_LINE_BYTES;
use atlasctl_protocol::msg::bench_event::{EventKind, Verdict, VerdictKind};

/// Put the child in its own process group, so a cancel can reach the whole
/// tree (`kill_group`) and not just the shell that fronted it.
///
/// # Errors
/// On a platform without process groups: a bench child that cannot be
/// cancelled as a unit must not be started at all.
#[cfg(unix)]
pub fn in_own_process_group(cmd: &mut std::process::Command) -> Result<&mut std::process::Command> {
    use std::os::unix::process::CommandExt;
    Ok(cmd.process_group(0))
}

#[cfg(not(unix))]
pub fn in_own_process_group(cmd: &mut std::process::Command) -> Result<&mut std::process::Command> {
    let _ = cmd;
    anyhow::bail!(
        "bench jobs run on Linux only: a child needs its own process group to be cancellable"
    )
}

/// `/proc/<pid>/stat` field 22, the process start time in clock ticks — the
/// thing that tells a live child from a pid the kernel reused.
pub fn start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.rsplit_once(')')?.1;
    after.split_whitespace().nth(19)?.parse().ok()
}

/// Is the process with this pid the one we started?
pub fn same_process(pid: u32, ticks: u64) -> bool {
    start_ticks(pid) == Some(ticks)
}

pub fn sanitize(line: &str) -> String {
    let mut s: String = line
        .chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect();
    if s.len() > MAX_LOG_LINE_BYTES {
        let mut cut = MAX_LOG_LINE_BYTES;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// The verdict line Atlas prints: `  Pass: …` / `  Fail: …` / `  Info: …`.
pub fn parse_verdict(line: &str) -> Option<Verdict> {
    let t = line.trim_start();
    for (prefix, kind) in [
        ("Pass: ", VerdictKind::Pass),
        ("Fail: ", VerdictKind::Fail),
        ("Info: ", VerdictKind::Info),
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(Verdict {
                kind,
                text: sanitize(rest),
            });
        }
    }
    None
}

/// `  [  12.3s] phase [n/m]` → a progress event.
pub fn parse_progress(line: &str) -> Option<EventKind> {
    let t = line.trim_start();
    let rest = t.strip_prefix('[')?;
    let (_, after) = rest.split_once("s]")?;
    let phase = after.trim();
    (!phase.is_empty()).then(|| EventKind::Progress {
        phase: sanitize(phase),
        detail: String::new(),
    })
}

pub fn kill_group(pid: u32, signal: &str) {
    let _ = std::process::Command::new("kill")
        .args([signal, "--", &format!("-{pid}")])
        .status();
    let _ = std::process::Command::new("kill")
        .args([signal, &pid.to_string()])
        .status();
}
