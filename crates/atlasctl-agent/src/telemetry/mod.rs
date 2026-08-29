// SPDX-License-Identifier: AGPL-3.0-only

//! Live numbers about a running model and the machine underneath it.

pub mod meminfo;
pub mod nvidia;

use atlasctl_core::io::ProcessRunner;
use atlasctl_protocol::telemetry::{DeviceStats, TelemetryCaps};

/// Clock below this while work is in flight counts as clamped.
///
/// The number matters. A GB10 idles around 200 MHz and runs above 2000 under
/// load; a clamp at 513 MHz once made every throughput measurement read two to
/// three times low while every correctness check stayed green. A threshold in
/// between catches that without firing at idle.
pub const DEFAULT_HEALTHY_CLOCK_MHZ: u32 = 1500;

/// Ask the machine what it can actually answer.
///
/// Run once at startup. Capabilities are earned by a reading, never assumed:
/// on unified-memory hardware `nvidia-smi` returns `[N/A]` for memory, and a
/// dashboard that assumed otherwise would render zeros as measurements.
pub fn probe(runner: &dyn ProcessRunner) -> TelemetryCaps {
    let Ok(out) = runner.run(&nvidia::argv()) else {
        return TelemetryCaps::default();
    };
    if !out.success() {
        return TelemetryCaps::default();
    }
    let reading = nvidia::parse(out.stdout.lines().next().unwrap_or_default());

    TelemetryCaps {
        gpu_util: reading.util_pct.is_some(),
        sm_clock: reading.sm_clock_mhz.is_some(),
        temperature: reading.temperature_c.is_some(),
        power: reading.power_w.is_some(),
        framebuffer_memory: reading.memory_total_bytes.is_some(),
        // Host memory is the meaningful figure when there is no framebuffer.
        unified_memory: meminfo::available(),
        engine_metrics: false,
        sm_clock_healthy_mhz: reading.sm_clock_mhz.map(|_| DEFAULT_HEALTHY_CLOCK_MHZ),
    }
}

/// What this machine's accelerator calls itself, for display.
///
/// Already parsed out of the capability probe's own output — `name` is the
/// first field of the query — so this costs one extra invocation at startup
/// and no new vendor-specific code. The fleet showed an empty string here
/// while the reading it came from said "NVIDIA GB10".
///
/// `None` when there is no accelerator, or when it declines to name itself.
/// An empty string is not a name, and rendering one puts a separator with
/// nothing after it in the interface.
pub fn accelerator_name(runner: &dyn ProcessRunner) -> Option<String> {
    let out = runner.run(&nvidia::argv()).ok()?;
    if !out.success() {
        return None;
    }
    nvidia::parse(out.stdout.lines().next().unwrap_or_default())
        .name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
}

/// Take one device sample, reporting only what the capabilities allow.
///
/// `busy` says whether the engine has work in flight, which is what makes a low
/// clock meaningful: 208 MHz at idle is correct, and the same figure under load
/// is a problem.
pub fn sample_device(runner: &dyn ProcessRunner, caps: &TelemetryCaps, busy: bool) -> DeviceStats {
    let reading = runner
        .run(&nvidia::argv())
        .ok()
        .filter(|o| o.success())
        .map(|o| nvidia::parse(o.stdout.lines().next().unwrap_or_default()))
        .unwrap_or_default();

    let clamped = match (busy, reading.sm_clock_mhz, caps.sm_clock_healthy_mhz) {
        (true, Some(mhz), Some(healthy)) => mhz < healthy,
        _ => false,
    };

    let (total, used, unified) = if caps.framebuffer_memory {
        (reading.memory_total_bytes, reading.memory_used_bytes, false)
    } else if caps.unified_memory {
        let m = meminfo::read();
        (m.total_bytes, m.used_bytes, true)
    } else {
        (None, None, false)
    };

    DeviceStats {
        gpu_util_pct: caps.gpu_util.then_some(reading.util_pct).flatten(),
        sm_clock_mhz: caps.sm_clock.then_some(reading.sm_clock_mhz).flatten(),
        sm_clock_clamped: clamped,
        temperature_c: caps.temperature.then_some(reading.temperature_c).flatten(),
        power_w: caps.power.then_some(reading.power_w).flatten(),
        memory_total_bytes: total,
        memory_used_bytes: used,
        memory_is_unified: unified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    /// Captured from a real DGX Spark.
    const GB10: &str = "NVIDIA GB10, [N/A], [N/A], 0 %, 208 MHz, 50, 5.24 W";

    fn runner_with(stdout: &str, status: i32) -> RecordingRunner {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status,
            stdout: stdout.into(),
            stderr: String::new(),
        });
        r
    }

    #[test]
    fn probing_a_gb10_finds_no_framebuffer_but_everything_else() {
        let caps = probe(&runner_with(GB10, 0));
        assert!(caps.gpu_util && caps.sm_clock && caps.temperature && caps.power);
        assert!(!caps.framebuffer_memory, "this part reports no framebuffer");
        assert_eq!(caps.sm_clock_healthy_mhz, Some(DEFAULT_HEALTHY_CLOCK_MHZ));
    }

    #[test]
    fn probing_a_machine_without_nvidia_smi_claims_nothing() {
        let caps = probe(&runner_with("", 127));
        assert!(!caps.gpu_util);
        assert!(!caps.sm_clock);
        assert_eq!(caps.sm_clock_healthy_mhz, None);
    }

    #[test]
    fn an_idle_low_clock_is_not_reported_as_clamped() {
        // 208 MHz with nothing running is correct, not a fault.
        let caps = probe(&runner_with(GB10, 0));
        let d = sample_device(&runner_with(GB10, 0), &caps, false);
        assert_eq!(d.sm_clock_mhz, Some(208));
        assert!(!d.sm_clock_clamped, "idle must not raise a clamp warning");
    }

    #[test]
    fn the_same_clock_under_load_is_reported_as_clamped() {
        // This is the failure that hid for weeks: throughput reads low while
        // every correctness check stays green.
        let caps = probe(&runner_with(GB10, 0));
        let clamped = "NVIDIA GB10, [N/A], [N/A], 96 %, 513 MHz, 71, 42.0 W";
        let d = sample_device(&runner_with(clamped, 0), &caps, true);
        assert!(
            d.sm_clock_clamped,
            "a clamped clock under load must be flagged"
        );
    }

    #[test]
    fn a_healthy_clock_under_load_is_not_flagged() {
        let caps = probe(&runner_with(GB10, 0));
        let healthy = "NVIDIA GB10, [N/A], [N/A], 96 %, 2405 MHz, 71, 92.0 W";
        assert!(!sample_device(&runner_with(healthy, 0), &caps, true).sm_clock_clamped);
    }

    #[test]
    fn memory_on_a_unified_part_comes_from_the_host_and_says_so() {
        let caps = probe(&runner_with(GB10, 0));
        let d = sample_device(&runner_with(GB10, 0), &caps, false);
        if caps.unified_memory {
            assert!(
                d.memory_is_unified,
                "the client must be told this is not VRAM"
            );
            assert!(d.memory_total_bytes.unwrap_or(0) > 0);
        }
    }

    #[test]
    fn a_capability_that_was_not_probed_is_never_reported() {
        // Even if a later reading contains the field, an unprobed capability
        // stays absent — otherwise the dashboard would flicker into life.
        let caps = TelemetryCaps::default();
        let d = sample_device(&runner_with(GB10, 0), &caps, true);
        assert_eq!(d.gpu_util_pct, None);
        assert_eq!(d.sm_clock_mhz, None);
        assert!(!d.sm_clock_clamped);
    }
}
