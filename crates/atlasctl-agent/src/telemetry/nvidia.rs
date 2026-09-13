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

// ── The bench surface's questions ──────────────────────────────────────────
//
// What a bench node reports about its accelerator beyond the per-second
// sample above: identity, the CUDA version, the clock ceiling, the thermal
// clock-event reasons, and the compute apps holding the device. Each is one
// `nvidia-smi` invocation, kept here — the provider module — so no neutral
// module carries the vendor's tool name. Parsing is separate from running,
// and tested on captured output.

/// Run `nvidia-smi` with `args`; `None` when it is absent or unhappy.
fn run(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("nvidia-smi")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `(name, driver_version, count)` of the first GPU.
pub fn identity() -> Option<(String, String, u32)> {
    let text = run(&[
        "--query-gpu=name,driver_version,count",
        "--format=csv,noheader",
    ])?;
    parse_identity(&text)
}

pub fn parse_identity(csv: &str) -> Option<(String, String, u32)> {
    let mut parts = csv.lines().next()?.split(',').map(str::trim);
    let name = parts.next()?.to_string();
    let driver = parts.next()?.to_string();
    let count = parts.next()?.parse().unwrap_or(1);
    Some((name, driver, count))
}

/// The CUDA version the driver header names (`13.0`); empty when unknown.
pub fn cuda_version() -> String {
    run(&[])
        .as_deref()
        .and_then(parse_cuda_version)
        .unwrap_or_default()
}

pub fn parse_cuda_version(header: &str) -> Option<String> {
    let i = header.find("CUDA Version:")?;
    header[i + 13..]
        .split_whitespace()
        .next()
        .map(str::to_owned)
}

/// `clocks.max.sm` of the first GPU, MHz.
pub fn clock_max_mhz() -> Option<f64> {
    run(&["--query-gpu=clocks.max.sm", "--format=csv,noheader,nounits"])
        .as_deref()
        .and_then(parse_clock_max)
}

pub fn parse_clock_max(csv: &str) -> Option<f64> {
    let n: f64 = csv.lines().next()?.trim().parse().ok()?;
    (n > 0.0).then_some(n)
}

/// Whether any THERMAL reason under "Clocks Event Reasons" is `Active`.
/// `None` when the block is absent. SW Power Capping is excluded on
/// purpose: on GB10 it is the steady state of a power-limited part.
pub fn throttle_thermal() -> Option<bool> {
    run(&["-q", "-d", "PERFORMANCE"])
        .as_deref()
        .and_then(parse_throttle_thermal)
}

pub fn parse_throttle_thermal(text: &str) -> Option<bool> {
    let mut in_reasons = false;
    let mut seen = false;
    let mut any = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Clocks Event Reasons Counters") {
            in_reasons = false;
            continue;
        }
        if t.starts_with("Clocks Event Reasons") {
            in_reasons = true;
            continue;
        }
        let Some((key, raw)) = t.split_once(':') else {
            if !t.is_empty() {
                in_reasons = false;
            }
            continue;
        };
        if !in_reasons {
            continue;
        }
        let active = match raw.trim() {
            "Active" => true,
            "Not Active" => false,
            _ => continue,
        };
        if matches!(
            key.trim(),
            "SW Thermal Slowdown" | "HW Thermal Slowdown" | "HW Power Brake Slowdown"
        ) {
            seen = true;
            any |= active;
        }
    }
    seen.then_some(any)
}

/// `pid, process_name` of every compute app on the device, one per line.
pub fn compute_apps() -> Vec<String> {
    run(&[
        "--query-compute-apps=pid,process_name",
        "--format=csv,noheader",
    ])
    .map(|text| {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod bench_probe_tests {
    use super::*;

    #[test]
    fn the_thermal_reasons_are_read_from_the_reasons_block_only() {
        // Verbatim from `nvidia-smi -q -d PERFORMANCE` on a GB10 (driver
        // 580), with HW Thermal Slowdown flipped to Active.
        let text = "\
    Clocks Event Reasons
        Idle                                           : Not Active
        Applications Clocks Setting                    : Not Active
        SW Power Cap                                   : Active
        HW Slowdown                                    : Not Active
            HW Thermal Slowdown                        : Active
            HW Power Brake Slowdown                    : Not Active
        Sync Boost                                     : Not Active
        SW Thermal Slowdown                            : Not Active
    Clocks Event Reasons Counters
        SW Thermal Slowdown                            : 12 us
";
        assert_eq!(parse_throttle_thermal(text), Some(true));
        let cool = text.replace(
            "HW Thermal Slowdown                        : Active",
            "HW Thermal Slowdown                        : Not Active",
        );
        // NEGATIVE CONTROL for the spelling: the counters-block name is not
        // the reasons-block name, and a fixture with the wrong one would pass
        // by never matching.
        assert_eq!(
            parse_throttle_thermal(
                "    Clocks Event Reasons\n        HW Power Brake Slowdown : Active\n"
            ),
            Some(true)
        );
        // SW power capping alone is not a thermal alert.
        assert_eq!(parse_throttle_thermal(&cool), Some(false));
        // NEGATIVE CONTROL: no reasons block at all is unknown, not false.
        assert_eq!(parse_throttle_thermal("    Performance State : P0\n"), None);
        // A counters block is not read as reasons.
        let only_counters =
            "    Clocks Event Reasons Counters\n        HW Thermal Slowdown : 5 us\n";
        assert_eq!(parse_throttle_thermal(only_counters), None);
    }

    #[test]
    fn identity_cuda_and_clock_ceiling_parse_from_captured_output() {
        assert_eq!(
            parse_identity("NVIDIA GB10, 580.126.09, 1\n"),
            Some(("NVIDIA GB10".into(), "580.126.09".into(), 1))
        );
        assert_eq!(parse_identity(""), None);
        assert_eq!(
            parse_cuda_version(
                "| NVIDIA-SMI 580.126.09    Driver Version: 580.126.09    CUDA Version: 13.0     |"
            ),
            Some("13.0".into())
        );
        assert_eq!(parse_cuda_version("no header"), None);
        assert_eq!(parse_clock_max("3003\n"), Some(3003.0));
        assert_eq!(parse_clock_max("[N/A]\n"), None);
        assert_eq!(parse_clock_max("0\n"), None);
    }
}
