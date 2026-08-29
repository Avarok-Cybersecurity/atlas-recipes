// SPDX-License-Identifier: AGPL-3.0-only

//! Parsing `nvidia-smi` CSV output.
//!
//! Every field is optional, because on a GB10 several of them genuinely are.
//! Real output from that hardware:
//!
//! ```text
//! NVIDIA GB10, [N/A], [N/A], 0 %, 208 MHz, 50, 5.24 W
//! ```
//!
//! The two `[N/A]`s are the memory fields. Grace-Blackwell is unified memory,
//! so there is no framebuffer to report — the pool is the host's, and it comes
//! from `/proc/meminfo` instead.

/// The query we ask for, in this order.
pub const QUERY: &str =
    "name,memory.total,memory.used,utilization.gpu,clocks.current.sm,temperature.gpu,power.draw";

/// Arguments for a telemetry sample.
pub fn argv() -> Vec<String> {
    vec![
        "nvidia-smi".into(),
        format!("--query-gpu={QUERY}"),
        "--format=csv,noheader".into(),
    ]
}

/// One accelerator's readings, as far as it reports them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reading {
    /// Device name.
    pub name: Option<String>,
    /// Framebuffer total, bytes. Absent on unified-memory parts.
    pub memory_total_bytes: Option<u64>,
    /// Framebuffer in use, bytes. Absent on unified-memory parts.
    pub memory_used_bytes: Option<u64>,
    /// Utilisation, percent.
    pub util_pct: Option<f64>,
    /// Current clock, MHz.
    pub sm_clock_mhz: Option<u32>,
    /// Temperature, Celsius.
    pub temperature_c: Option<f64>,
    /// Power draw, Watts.
    pub power_w: Option<f64>,
}

/// A field nvidia-smi declines to answer.
///
/// It uses several spellings depending on why, and they all mean the same
/// thing to us: there is no number here. Treating any of them as a value is
/// how a dashboard ends up reporting zeros as measurements.
fn absent(raw: &str) -> bool {
    let t = raw.trim();
    t.is_empty()
        || t.eq_ignore_ascii_case("[N/A]")
        || t.eq_ignore_ascii_case("N/A")
        || t.eq_ignore_ascii_case("[Not Supported]")
        || t.eq_ignore_ascii_case("[Unknown Error]")
        || t.eq_ignore_ascii_case("[Insufficient Permissions]")
}

/// Strip a trailing unit and parse the number in front of it.
fn number(raw: &str) -> Option<f64> {
    if absent(raw) {
        return None;
    }
    raw.split_whitespace().next()?.parse::<f64>().ok()
}

/// Parse one line of CSV output.
pub fn parse(line: &str) -> Reading {
    let f: Vec<&str> = line.split(',').collect();
    let at = |i: usize| f.get(i).copied().unwrap_or("");
    let mib_to_bytes = |v: f64| (v * 1024.0 * 1024.0) as u64;

    Reading {
        name: (!absent(at(0))).then(|| at(0).trim().to_string()),
        memory_total_bytes: number(at(1)).map(mib_to_bytes),
        memory_used_bytes: number(at(2)).map(mib_to_bytes),
        util_pct: number(at(3)),
        sm_clock_mhz: number(at(4)).map(|v| v as u32),
        temperature_c: number(at(5)),
        power_w: number(at(6)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a real DGX Spark, not invented.
    const GB10: &str = "NVIDIA GB10, [N/A], [N/A], 0 %, 208 MHz, 50, 5.24 W";

    #[test]
    fn a_real_gb10_line_parses_with_no_memory_and_everything_else_present() {
        let r = parse(GB10);
        assert_eq!(r.name.as_deref(), Some("NVIDIA GB10"));
        // The whole reason capabilities are probed: this part has no framebuffer.
        assert_eq!(r.memory_total_bytes, None);
        assert_eq!(r.memory_used_bytes, None);
        assert_eq!(r.util_pct, Some(0.0));
        assert_eq!(r.sm_clock_mhz, Some(208));
        assert_eq!(r.temperature_c, Some(50.0));
        assert_eq!(r.power_w, Some(5.24));
    }

    #[test]
    fn a_discrete_card_reports_its_framebuffer() {
        let r = parse("NVIDIA A100, 40960 MiB, 1024 MiB, 73 %, 1410 MHz, 61, 250.5 W");
        assert_eq!(r.memory_total_bytes, Some(40960 * 1024 * 1024));
        assert_eq!(r.memory_used_bytes, Some(1024 * 1024 * 1024));
        assert_eq!(r.util_pct, Some(73.0));
    }

    #[test]
    fn every_spelling_of_unavailable_is_treated_as_absent() {
        for spelling in [
            "[N/A]",
            "N/A",
            "[Not Supported]",
            "[Unknown Error]",
            "  ",
            "",
        ] {
            let line = format!(
                "GPU, {spelling}, {spelling}, {spelling}, {spelling}, {spelling}, {spelling}"
            );
            let r = parse(&line);
            assert_eq!(r.util_pct, None, "{spelling:?} should be absent");
            assert_eq!(r.sm_clock_mhz, None, "{spelling:?} should be absent");
            assert_eq!(r.power_w, None, "{spelling:?} should be absent");
        }
    }

    #[test]
    fn a_truncated_line_does_not_panic_and_yields_absences() {
        let r = parse("NVIDIA GB10");
        assert_eq!(r.name.as_deref(), Some("NVIDIA GB10"));
        assert_eq!(r.util_pct, None);
    }

    #[test]
    fn empty_output_yields_nothing_rather_than_defaults_that_look_real() {
        assert_eq!(parse(""), Reading::default());
    }
}
