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
        // f64 because it crosses the wire as JSON, where there is no u64.
        #[expect(
            clippy::cast_precision_loss,
            reason = "free space is displayed in gigabytes"
        )]
        let disk = atlasctl_core::platform::free_bytes(&self.disk_path).map(|b| b as f64);
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

/// What this machine is currently serving.
///
/// Read from the container runtime rather than remembered from a launch this
/// process performed, so an agent that was restarted still reports the model it
/// left running. An agent that forgot its own containers on restart would
/// report an idle machine that is in fact holding a GPU.
pub trait RunningSource: Send + Sync {
    /// The recipe running here, if any.
    fn running(&self) -> Option<String>;
}

/// Argv that lists the recipes of managed containers still running.
///
/// Filtered by our own label, so an operator's unrelated containers are neither
/// listed nor confused for ours.
#[must_use]
pub fn running_probe_argv() -> Vec<String> {
    vec![
        "docker".into(),
        "ps".into(),
        "--filter".into(),
        format!(
            "label={}=1",
            atlasctl_core::docker::translate::LABEL_MANAGED
        ),
        "--format".into(),
        format!(
            "{{{{.Label \"{}\"}}}}",
            atlasctl_core::docker::translate::LABEL_RECIPE
        ),
    ]
}

/// Reads it from the container runtime.
pub struct DockerRunning(pub std::sync::Arc<dyn atlasctl_core::io::ProcessRunner>);

impl RunningSource for DockerRunning {
    fn running(&self) -> Option<String> {
        let out = self.0.run(&running_probe_argv()).ok()?;
        if !out.success() {
            return None;
        }
        // First non-empty line. More than one managed container can be up — a
        // cluster rank plus a solo launch — and naming one is better than
        // naming none; the launch surface shows the full picture.
        out.stdout
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod running_tests {
    use super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;
    use std::sync::Arc;

    /// An operator's own containers must not be listed as ours, nor ours
    /// confused for theirs.
    #[test]
    fn the_probe_asks_only_for_containers_we_manage() {
        let argv = running_probe_argv();
        assert_eq!(argv[0], "docker");
        assert!(
            argv.iter().any(|a| a.contains("io.atlasctl.managed=1")),
            "{argv:?}"
        );
        assert!(
            argv.iter().any(|a| a.contains("io.atlasctl.recipe")),
            "{argv:?}"
        );
    }

    #[test]
    fn a_running_recipe_is_reported() {
        let r = Arc::new(RecordingRunner::new());
        r.push_result(Output {
            status: 0,
            stdout: "qwen3.6-35b-a3b-fp8-bf16head\n".to_owned(),
            stderr: String::new(),
        });
        let src = DockerRunning(Arc::clone(&r) as Arc<dyn atlasctl_core::io::ProcessRunner>);
        assert_eq!(
            src.running().as_deref(),
            Some("qwen3.6-35b-a3b-fp8-bf16head")
        );
    }

    #[test]
    fn nothing_running_is_reported_as_nothing() {
        let r = Arc::new(RecordingRunner::new());
        let src = DockerRunning(Arc::clone(&r) as Arc<dyn atlasctl_core::io::ProcessRunner>);
        assert_eq!(src.running(), None);
    }

    /// A container runtime that is not answering is not the same as an idle
    /// machine, and neither is reported as a recipe.
    #[test]
    fn an_unavailable_runtime_reports_nothing_rather_than_garbage() {
        let r = Arc::new(RecordingRunner::new());
        r.push_result(Output {
            status: 1,
            stdout: String::new(),
            stderr: "Cannot connect to the Docker daemon".to_owned(),
        });
        let src = DockerRunning(Arc::clone(&r) as Arc<dyn atlasctl_core::io::ProcessRunner>);
        assert_eq!(src.running(), None);
    }

    /// Blank lines come back for a container whose label is missing; they are
    /// not a recipe name.
    #[test]
    fn blank_lines_are_not_recipe_names() {
        let r = Arc::new(RecordingRunner::new());
        r.push_result(Output {
            status: 0,
            stdout: "\n\n  \nreal-recipe\n".to_owned(),
            stderr: String::new(),
        });
        let src = DockerRunning(Arc::clone(&r) as Arc<dyn atlasctl_core::io::ProcessRunner>);
        assert_eq!(src.running().as_deref(), Some("real-recipe"));
    }
}
