// SPDX-License-Identifier: AGPL-3.0-only

//! The live facts a speed-class equivalence check needs, read from the same
//! three places Atlas's own hardware collector reads them: `nvidia-smi` for
//! the clock ceiling and the clock-event reasons, sysfs for the chassis
//! zones, procfs for memory. Facts only; nothing here decides anything.

use atlasctl_protocol::msg::bench_node::HostThermal;
use std::path::Path;

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

/// `clocks.max.sm` for the first GPU, MHz.
pub fn parse_clock_max(csv: &str) -> Option<f64> {
    let first = csv.lines().next()?.trim();
    let n: f64 = first.parse().ok()?;
    (n > 0.0).then_some(n)
}

/// Whether any thermal reason under "Clocks Event Reasons" is `Active`.
///
/// `None` when the block is absent. SW Power Capping is excluded on purpose:
/// on GB10 it is the steady state of a power-limited part and says nothing
/// about a thermal fault.
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
            "SW Thermal Slowdown" | "HW Thermal Slowdown" | "HW Power Braking"
        ) {
            seen = true;
            any |= active;
        }
    }
    seen.then_some(any)
}

/// Every `thermal_zone*/temp` under `root`, °C, in numeric zone order.
pub fn read_zones(root: &Path) -> Vec<f64> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut zones: Vec<(u32, f64)> = dir
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n: u32 = name.strip_prefix("thermal_zone")?.parse().ok()?;
            let raw = std::fs::read_to_string(e.path().join("temp")).ok()?;
            let milli: f64 = raw.trim().parse().ok()?;
            Some((n, milli / 1000.0))
        })
        .collect();
    zones.sort_by_key(|(n, _)| *n);
    zones.into_iter().map(|(_, t)| t).collect()
}

/// Collect the four facts.
pub fn collect() -> HostThermal {
    HostThermal {
        chassis_temps_c: read_zones(Path::new("/sys/class/thermal")),
        throttle_thermal: run(&["-q", "-d", "PERFORMANCE"])
            .as_deref()
            .and_then(parse_throttle_thermal),
        sm_clock_max_mhz: run(&["--query-gpu=clocks.max.sm", "--format=csv,noheader,nounits"])
            .as_deref()
            .and_then(parse_clock_max),
        mem_total_kb: crate::telemetry::meminfo::read()
            .total_bytes
            .map(|b| b / 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thermal_reasons_are_read_from_the_reasons_block_only() {
        let text = "\
    Clocks Event Reasons
        Idle                              : Not Active
        SW Power Capping                  : Active
        SW Thermal Slowdown               : Not Active
        HW Thermal Slowdown               : Active
        HW Power Braking                  : Not Active
    Clocks Event Reasons Counters
        SW Thermal Slowdown               : 12 us
";
        assert_eq!(parse_throttle_thermal(text), Some(true));
        let cool = text.replace(
            "HW Thermal Slowdown               : Active",
            "HW Thermal Slowdown               : Not Active",
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
    fn the_clock_ceiling_and_zones_parse_as_numbers_or_not_at_all() {
        assert_eq!(parse_clock_max("3003\n"), Some(3003.0));
        assert_eq!(parse_clock_max("[N/A]\n"), None);
        assert_eq!(parse_clock_max("0\n"), None);
        let dir = std::env::temp_dir().join(format!("zones-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (n, t) in [(0, "65000"), (10, "59000"), (2, "notanumber"), (1, "62500")] {
            let z = dir.join(format!("thermal_zone{n}"));
            std::fs::create_dir_all(&z).unwrap();
            std::fs::write(z.join("temp"), t).unwrap();
        }
        std::fs::create_dir_all(dir.join("cooling_device0")).unwrap();
        // Numeric zone order (0, 1, 10), the unreadable one dropped.
        assert_eq!(read_zones(&dir), vec![65.0, 62.5, 59.0]);
        assert!(read_zones(&dir.join("nope")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
