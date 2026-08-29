// SPDX-License-Identifier: AGPL-3.0-only

//! Host memory, which on unified-memory hardware is the meaningful figure.
//!
//! A GB10's 119.7 GB "GPU memory" is the host's LPDDR5X: `nvidia-smi` reports
//! no framebuffer at all, so this is where the number comes from.

/// Total and in-use memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemInfo {
    /// Total, bytes.
    pub total_bytes: Option<u64>,
    /// In use, bytes — total minus what is actually available.
    pub used_bytes: Option<u64>,
}

/// Whether host memory can be read here.
#[cfg(not(windows))]
pub fn available() -> bool {
    std::path::Path::new("/proc/meminfo").exists()
}

/// Whether host memory can be read here.
///
/// Always: `GlobalMemoryStatusEx` is part of the OS, not an optional file.
#[cfg(windows)]
pub fn available() -> bool {
    true
}

/// Read host memory.
#[cfg(not(windows))]
pub fn read() -> MemInfo {
    std::fs::read_to_string("/proc/meminfo")
        .map(|s| parse(&s))
        .unwrap_or_default()
}

/// Read host memory.
///
/// `ullAvailPhys` is the counterpart of `MemAvailable`, not of `MemFree`: it
/// counts memory obtainable without paging, so a machine that has just loaded
/// a large model reads as busy rather than as full.
#[cfg(windows)]
pub fn read() -> MemInfo {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut st: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    // Required by the API: it dispatches on the struct size it was compiled
    // against, and a zero here makes the call fail rather than misbehave.
    st.dwLength = u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0);
    // SAFETY: `st` is a correctly sized, zeroed MEMORYSTATUSEX with dwLength
    // set, which is the entire contract; it is read only after success.
    if unsafe { GlobalMemoryStatusEx(&raw mut st) } == 0 {
        return MemInfo::default();
    }
    MemInfo {
        total_bytes: Some(st.ullTotalPhys),
        used_bytes: Some(st.ullTotalPhys.saturating_sub(st.ullAvailPhys)),
    }
}

/// Parse `/proc/meminfo`.
///
/// Uses `MemAvailable` rather than `MemFree`: free memory excludes reclaimable
/// page cache, so on a box that has just loaded 100 GB of weights it reads
/// alarmingly low while the memory is in fact obtainable.
pub fn parse(body: &str) -> MemInfo {
    let field = |name: &str| -> Option<u64> {
        body.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024)
    };
    let total = field("MemTotal:");
    let avail = field("MemAvailable:");
    MemInfo {
        total_bytes: total,
        used_bytes: match (total, avail) {
            (Some(t), Some(a)) => Some(t.saturating_sub(a)),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a real DGX Spark.
    const REAL: &str =
        "MemTotal:       127601452 kB\nMemFree:        45557560 kB\nMemAvailable:   115669068 kB\n";

    #[test]
    fn a_real_gb10_meminfo_parses_to_roughly_119_gib() {
        let m = parse(REAL);
        let gib = m.total_bytes.unwrap() as f64 / (1024.0 * 1024.0 * 1024.0);
        assert!((118.0..=123.0).contains(&gib), "got {gib} GiB");
    }

    #[test]
    fn used_is_derived_from_available_not_free() {
        // MemFree would say ~78 GiB used on this box; MemAvailable says ~11.
        // The second is the honest figure, because page cache is reclaimable.
        let m = parse(REAL);
        let used_gib = m.used_bytes.unwrap() as f64 / (1024.0 * 1024.0 * 1024.0);
        assert!(
            used_gib < 20.0,
            "used should follow MemAvailable, got {used_gib} GiB"
        );
    }

    #[test]
    fn missing_fields_yield_absences_rather_than_zeros() {
        assert_eq!(parse(""), MemInfo::default());
        assert_eq!(parse("MemTotal:  100 kB\n").used_bytes, None);
    }
}
