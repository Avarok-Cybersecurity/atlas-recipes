// SPDX-License-Identifier: AGPL-3.0-only

//! The live facts a speed-class equivalence check needs, read from the same
//! three places Atlas's own hardware collector reads them: the accelerator
//! provider (`telemetry::nvidia`) for the clock ceiling and the clock-event
//! reasons, sysfs for the chassis zones, procfs for memory. Facts only;
//! nothing here decides anything.

use atlasctl_protocol::msg::bench_node::HostThermal;
use std::path::Path;

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
        throttle_thermal: crate::telemetry::nvidia::throttle_thermal(),
        sm_clock_max_mhz: crate::telemetry::nvidia::clock_max_mhz(),
        mem_total_kb: crate::telemetry::meminfo::read()
            .total_bytes
            .map(|b| b / 1024),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zones_parse_as_numbers_or_not_at_all() {
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
