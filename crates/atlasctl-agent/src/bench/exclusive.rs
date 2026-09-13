// SPDX-License-Identifier: AGPL-3.0-only

//! Why the box cannot start a gate right now.
//!
//! One gate per box: it serves a model that takes most of unified memory,
//! and a speed number measured next to anything else is not a measurement.
//! Every probe here is a fact about the machine, read fresh, and the answer
//! names what is in the way so the operator looks in the right place.

use anyhow::Result;
use std::path::Path;

/// One reason the box is busy, or short of a resource.
#[derive(Debug, Clone, PartialEq)]
pub enum Busy {
    /// A `spark` process, ours or not.
    Spark(Vec<u32>),
    /// A GPU compute app the driver reports.
    GpuApp(String),
    /// Another job of ours, not yet terminal.
    Job(String),
    Memory {
        available_frac: f64,
        required_frac: f64,
    },
    Disk {
        free_bytes: u64,
        required_bytes: u64,
    },
}

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spark(p) => write!(
                f,
                "spark pid {}",
                p.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
            ),
            Self::GpuApp(s) => write!(f, "GPU compute app {s}"),
            Self::Job(j) => write!(f, "job {j} is running"),
            Self::Memory {
                available_frac,
                required_frac,
            } => write!(
                f,
                "{:.0} % of host memory available, {:.0} % needed",
                available_frac * 100.0,
                required_frac * 100.0
            ),
            Self::Disk {
                free_bytes,
                required_bytes,
            } => write!(
                f,
                "{} MiB free on the cache, {} MiB needed",
                free_bytes >> 20,
                required_bytes >> 20
            ),
        }
    }
}

/// The readings the decision is made from; gathered by [`probe`], judged by
/// [`judge`] so the rule is testable on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readings {
    pub spark_pids: Vec<u32>,
    pub gpu_apps: Vec<String>,
    pub running_job: Option<String>,
    pub free_fraction: Option<f64>,
    pub disk_free_bytes: Option<u64>,
}

/// Every reason not to start, in the order an operator should read them.
#[must_use]
pub fn judge(r: &Readings, min_free_fraction: f64, min_free_disk: u64) -> Vec<Busy> {
    let mut out = Vec::new();
    if let Some(j) = &r.running_job {
        out.push(Busy::Job(j.clone()));
    }
    if !r.spark_pids.is_empty() {
        out.push(Busy::Spark(r.spark_pids.clone()));
    }
    for a in &r.gpu_apps {
        out.push(Busy::GpuApp(a.clone()));
    }
    match r.free_fraction {
        Some(frac) if frac < min_free_fraction => out.push(Busy::Memory {
            available_frac: frac,
            required_frac: min_free_fraction,
        }),
        Some(_) => {}
        None => out.push(Busy::Memory {
            available_frac: 0.0,
            required_frac: min_free_fraction,
        }),
    }
    match r.disk_free_bytes {
        Some(free) if free < min_free_disk => out.push(Busy::Disk {
            free_bytes: free,
            required_bytes: min_free_disk,
        }),
        Some(_) => {}
        None => out.push(Busy::Disk {
            free_bytes: 0,
            required_bytes: min_free_disk,
        }),
    }
    out
}

/// Read the machine. `cache_dir` is where the disk floor applies.
pub fn probe(cache_dir: &Path, running_job: Option<String>) -> Readings {
    Readings {
        spark_pids: spark_pids(),
        gpu_apps: gpu_apps(),
        running_job,
        free_fraction: free_fraction(),
        disk_free_bytes: disk_free(cache_dir),
    }
}

/// `pgrep -x spark`: exact name, never a command-line substring.
pub fn spark_pids() -> Vec<u32> {
    let me = std::process::id();
    std::process::Command::new("pgrep")
        .args(["-x", "spark"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.trim().parse::<u32>().ok())
                .filter(|p| *p != me)
                .collect()
        })
        .unwrap_or_default()
}

/// The compute apps holding the device, `pid, process_name` per line.
pub fn gpu_apps() -> Vec<String> {
    crate::telemetry::nvidia::compute_apps()
}

/// `MemAvailable / MemTotal`, the same reading Atlas's self-start applies.
pub fn free_fraction() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<f64> {
        text.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<f64>()
            .ok()
    };
    let total = field("MemTotal:")?;
    let avail = field("MemAvailable:")?;
    (total > 0.0).then(|| avail / total)
}

/// Free bytes on the filesystem holding `dir` (created if absent).
pub fn disk_free(dir: &Path) -> Option<u64> {
    let _ = std::fs::create_dir_all(dir);
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail"])
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()?
        .trim()
        .parse()
        .ok()
}

/// Bytes free → a `Result` the caller can `?` into a refusal.
pub fn check(r: &Readings, min_free_fraction: f64, min_free_disk: u64) -> Result<()> {
    let busy = judge(r, min_free_fraction, min_free_disk);
    if busy.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{}",
            busy.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free() -> Readings {
        Readings {
            spark_pids: vec![],
            gpu_apps: vec![],
            running_job: None,
            free_fraction: Some(0.94),
            disk_free_bytes: Some(100 << 30),
        }
    }

    #[test]
    fn a_free_box_has_no_reason() {
        assert!(judge(&free(), 0.85, 20 << 30).is_empty());
    }

    /// Each reading flips exactly one reason, named.
    #[test]
    fn each_reading_flips_one_reason() {
        let mut r = free();
        r.spark_pids = vec![7];
        assert_eq!(judge(&r, 0.85, 1), vec![Busy::Spark(vec![7])]);
        let mut r = free();
        r.gpu_apps = vec!["1234, python".into()];
        assert_eq!(judge(&r, 0.85, 1).len(), 1);
        let mut r = free();
        r.running_job = Some("jb-1-x".into());
        assert!(matches!(judge(&r, 0.85, 1)[0], Busy::Job(_)));
        let mut r = free();
        r.free_fraction = Some(0.5);
        assert!(matches!(judge(&r, 0.85, 1)[0], Busy::Memory { .. }));
        let mut r = free();
        r.disk_free_bytes = Some(1);
        assert!(matches!(judge(&r, 0.85, 2)[0], Busy::Disk { .. }));
    }

    /// NEGATIVE CONTROL: an unreadable reading is a refusal, never a pass.
    #[test]
    fn unreadable_readings_refuse() {
        let mut r = free();
        r.free_fraction = None;
        assert!(matches!(judge(&r, 0.85, 1)[0], Busy::Memory { .. }));
        let mut r = free();
        r.disk_free_bytes = None;
        assert!(matches!(judge(&r, 0.85, 1)[0], Busy::Disk { .. }));
    }

    #[test]
    fn the_probes_run_on_this_box() {
        // Whatever they read, they must not panic, and the disk probe must
        // answer for a directory that exists.
        let dir = std::env::temp_dir();
        assert!(disk_free(&dir).is_some());
        let _ = spark_pids();
        let _ = free_fraction();
    }
}
