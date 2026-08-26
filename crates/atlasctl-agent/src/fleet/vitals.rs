// SPDX-License-Identifier: AGPL-3.0-only

//! Where this machine's numbers come from.
//!
//! Split from the fleet view for size. The rule it exists to hold is stated
//! here rather than assumed: a field the hardware cannot answer becomes
//! `Metric::Unsupported`, never zero. On a GB10 that is not a defensive
//! nicety — `nvidia-smi` genuinely answers N/A for every memory field, because
//! Grace-Blackwell has no framebuffer.

use atlasctl_protocol::fleet::NodeVitals;
use std::time::Instant;

/// Supplies this machine's vitals.
///
/// A trait rather than a call into the telemetry functions directly, so the
/// fleet view can be tested against a machine that answers everything, a
/// machine that answers nothing, and the GB10 case where the memory questions
/// have no answer at all.
pub trait VitalsSource: Send + Sync {
    /// The current sample.
    fn vitals(&self) -> NodeVitals;
}

/// Turns a device sample into node vitals.
///
/// The `Option -> Metric` conversion is where "absent" is preserved: a field
/// the hardware cannot answer becomes `Metric::Unsupported`, never zero.
#[must_use]
pub fn vitals_from_device(
    d: &atlasctl_protocol::telemetry::DeviceStats,
    disk_free_bytes: Option<f64>,
    docker_ok: bool,
    uptime_s: u64,
    healthy_clock_mhz: Option<u32>,
) -> NodeVitals {
    use atlasctl_protocol::fleet::Metric;
    let used_frac = match (d.memory_used_bytes, d.memory_total_bytes) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a memory fraction is displayed to two significant figures"
        )]
        (Some(u), Some(t)) if t > 0 => Metric::reading(u as f64 / t as f64),
        _ => Metric::Unsupported,
    };
    NodeVitals {
        accelerator_util: d.gpu_util_pct.into(),
        sm_clock_mhz: d.sm_clock_mhz.map(f64::from).into(),
        sm_clock_healthy_mhz: healthy_clock_mhz,
        temperature_c: d.temperature_c.into(),
        power_w: d.power_w.into(),
        memory_used_frac: used_frac,
        #[expect(
            clippy::cast_precision_loss,
            reason = "byte counts are displayed in gigabytes"
        )]
        memory_total_bytes: d.memory_total_bytes.map(|b| b as f64).into(),
        disk_free_bytes: disk_free_bytes.into(),
        docker_ok,
        agent_uptime_s: uptime_s,
    }
}

/// Vitals read from this machine.
///
/// Capabilities are probed once at construction, because the answer does not
/// change while the agent runs and re-probing every second would spawn a
/// process every second. Individual readings are taken per sample.
pub struct SystemVitals {
    runner: std::sync::Arc<dyn atlasctl_core::io::ProcessRunner>,
    caps: atlasctl_protocol::telemetry::TelemetryCaps,
    started: Instant,
    /// Filesystem whose free space matters — images and the model cache.
    disk_path: std::path::PathBuf,
}

impl std::fmt::Debug for SystemVitals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemVitals")
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

impl SystemVitals {
    /// Probe this machine's telemetry capabilities and start sampling.
    #[must_use]
    pub fn new(
        runner: std::sync::Arc<dyn atlasctl_core::io::ProcessRunner>,
        disk_path: std::path::PathBuf,
    ) -> Self {
        let caps = crate::telemetry::probe(runner.as_ref());
        Self {
            runner,
            caps,
            started: Instant::now(),
            disk_path,
        }
    }

    /// What this machine can answer.
    #[must_use]
    pub const fn caps(&self) -> &atlasctl_protocol::telemetry::TelemetryCaps {
        &self.caps
    }
}

impl VitalsSource for SystemVitals {
    fn vitals(&self) -> NodeVitals {
        // `busy` drives the clamp decision, and it is the agent's call rather
        // than the client's: an idle part at a low clock is normal, the same
        // clock under load is the failure that hides for weeks.
        let device = crate::telemetry::sample_device(self.runner.as_ref(), &self.caps, false);
        let disk = free_bytes(&self.disk_path);
        let docker_ok = self
            .runner
            .run(&docker_probe_argv())
            .is_ok_and(|o| o.success());
        vitals_from_device(
            &device,
            disk,
            docker_ok,
            self.started.elapsed().as_secs(),
            self.caps.sm_clock_healthy_mhz,
        )
    }
}

/// The one way this project asks whether the container runtime is answering.
///
/// `docker info` exposes `.ServerVersion`; `docker version` does not — its
/// field is `.Server.Version`, and asking `version` for `.ServerVersion` exits
/// non-zero with a template error on Docker 29. That is a silent way to report
/// a healthy daemon as unreachable, so there is exactly one definition of the
/// probe and both callers use it.
#[must_use]
pub fn docker_probe_argv() -> Vec<String> {
    vec![
        "docker".to_owned(),
        "info".to_owned(),
        "--format".to_owned(),
        "{{.ServerVersion}}".to_owned(),
    ]
}

/// Free bytes on the filesystem holding `path`, when it can be determined.
///
/// A full model cache is a leading cause of launch failure, so this is worth
/// reporting even though it needs a platform call.
fn free_bytes(path: &std::path::Path) -> Option<f64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        // The cache directory may not exist yet on a fresh install, and statvfs
        // on a missing path fails. What matters is the filesystem that WOULD
        // hold it, so walk up to the nearest ancestor that does exist.
        let mut probe = path;
        while !probe.exists() {
            match probe.parent() {
                Some(parent) => probe = parent,
                None => return None,
            }
        }
        let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated path and `stat` is written only
        // by the call, which reports success before we read it.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) } != 0 {
            return None;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "free space is displayed in gigabytes"
        )]
        Some(stat.f_bavail as f64 * stat.f_frsize as f64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
