// SPDX-License-Identifier: AGPL-3.0-only

//! Live numbers about a running model, and about the machine underneath it.
//!
//! Every field is optional and every capability is **probed, not assumed**.
//! That is not defensive style — it is required by the hardware. On a GB10,
//! `nvidia-smi` reports `N/A` for every memory field, because Grace-Blackwell
//! is unified memory: the "GPU memory" is the host's LPDDR5X. A dashboard that
//! assumed a framebuffer would render zeros and call them measurements.

use serde::{Deserialize, Serialize};

/// What this machine can actually answer.
///
/// Sent at the handshake so a client renders only tiles it has data for, and
/// shows an explicit "not available on this hardware" state for the rest rather
/// than a zero.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryCaps {
    /// Accelerator utilisation is readable.
    pub gpu_util: bool,
    /// Clock is readable.
    pub sm_clock: bool,
    /// Temperature is readable.
    pub temperature: bool,
    /// Power draw is readable.
    pub power: bool,
    /// A dedicated framebuffer is reported. False on unified-memory parts.
    pub framebuffer_memory: bool,
    /// Host memory is readable, which is the meaningful figure on unified parts.
    pub unified_memory: bool,
    /// The model server's metrics endpoint answered.
    pub engine_metrics: bool,
    /// Clock below this under load is reported as clamped.
    pub sm_clock_healthy_mhz: Option<u32>,
}

/// One sample.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Seconds since the launch started.
    pub uptime_s: u64,
    /// Engine-level numbers, when the metrics endpoint answered.
    pub engine: Option<EngineStats>,
    /// Accelerator and host numbers.
    pub device: DeviceStats,
}

/// Numbers from the model server's own metrics endpoint.
///
/// Scraped rather than parsed out of logs: the endpoint is a stable contract
/// that ships with the engine, whereas a log format is not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineStats {
    /// Requests in flight.
    pub requests_active: Option<u64>,
    /// Requests handled since start.
    pub requests_total: Option<u64>,
    /// Tokens generated since start.
    pub generation_tokens_total: Option<u64>,
    /// Prompt tokens processed since start.
    pub prompt_tokens_total: Option<u64>,
    /// Decode rate, derived from the change in generated tokens.
    ///
    /// Derived rather than read: the engine exposes a counter, not a rate.
    pub decode_tokens_per_s: Option<f64>,
    /// Median time to first token, from the histogram.
    pub ttft_p50_ms: Option<f64>,
    /// Tool calls handled since start.
    pub tool_calls_total: Option<u64>,
    /// Fraction of drafted tokens accepted, when speculating.
    pub draft_accept_rate: Option<f64>,
}

/// Numbers about the machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceStats {
    /// Accelerator utilisation, percent.
    pub gpu_util_pct: Option<f64>,
    /// Current clock, MHz.
    pub sm_clock_mhz: Option<u32>,
    /// Whether the clock is clamped while work is in flight.
    ///
    /// Worth its own field rather than leaving it to the client to infer: a
    /// clamped clock makes every throughput number read low while nothing
    /// else looks wrong, which is exactly the failure that hides for weeks.
    pub sm_clock_clamped: bool,
    /// Temperature, Celsius.
    pub temperature_c: Option<f64>,
    /// Power draw, Watts.
    pub power_w: Option<f64>,
    /// Total memory available to the machine, bytes.
    ///
    /// On a unified-memory part this is host memory, and the client should say
    /// so rather than labelling it VRAM.
    pub memory_total_bytes: Option<u64>,
    /// Memory in use, bytes.
    pub memory_used_bytes: Option<u64>,
    /// Whether the memory figures describe a unified pool.
    pub memory_is_unified: bool,
}

/// Where a launch has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum LaunchPhase {
    /// Fetching the image.
    Pulling,
    /// Container created, process starting.
    Starting,
    /// Loading weights. This takes minutes, so a client should show progress
    /// rather than a spinner.
    LoadingWeights,
    /// Health checks pass.
    Ready,
    /// Answering requests.
    Serving,
    /// Running but failing its health check.
    Degraded {
        /// What the check reported.
        reason: String,
    },
    /// Being stopped.
    Stopping,
    /// Gone.
    Exited {
        /// Exit status.
        code: Option<i32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_default_to_nothing_being_available() {
        // Everything must be earned by a probe. A default of "available" would
        // make an unprobed field render as a real measurement.
        let c = TelemetryCaps::default();
        assert!(!c.gpu_util && !c.framebuffer_memory && !c.engine_metrics);
        assert_eq!(c.sm_clock_healthy_mhz, None);
    }

    #[test]
    fn a_gb10_capability_set_reports_no_framebuffer_but_unified_memory() {
        // The shape this hardware actually produces.
        let c = TelemetryCaps {
            gpu_util: true,
            sm_clock: true,
            temperature: true,
            power: true,
            framebuffer_memory: false,
            unified_memory: true,
            engine_metrics: true,
            sm_clock_healthy_mhz: Some(1500),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<TelemetryCaps>(&json).unwrap(), c);
        assert!(!c.framebuffer_memory, "GB10 reports no framebuffer");
    }

    #[test]
    fn absent_measurements_serialize_as_null_rather_than_zero() {
        let s = Stats::default();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""gpu_util_pct":null"#), "{json}");
        assert!(
            !json.contains(r#""gpu_util_pct":0"#),
            "an absent reading must not read as zero"
        );
    }

    #[test]
    fn phases_round_trip_including_their_payloads() {
        for p in [
            LaunchPhase::Pulling,
            LaunchPhase::LoadingWeights,
            LaunchPhase::Serving,
            LaunchPhase::Degraded {
                reason: "health check failed".into(),
            },
            LaunchPhase::Exited { code: Some(137) },
        ] {
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(
                serde_json::from_str::<LaunchPhase>(&json).unwrap(),
                p,
                "{json}"
            );
        }
    }

    #[test]
    fn a_clamped_clock_is_its_own_signal() {
        let d = DeviceStats {
            sm_clock_mhz: Some(513),
            sm_clock_clamped: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""sm_clock_clamped":true"#), "{json}");
    }
}
