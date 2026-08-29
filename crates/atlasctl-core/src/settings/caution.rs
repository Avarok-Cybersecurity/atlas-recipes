// SPDX-License-Identifier: AGPL-3.0-only

//! Values that are allowed and still worth a word before you run them.
//!
//! Separate from the bounds because they answer different questions. A bound
//! says "this cannot work"; a caution says "this can work, and on your hardware
//! it has not". Turning the second into the first would block machines where
//! the value is fine — the ceiling that is dangerous on a GB10's unified memory
//! is not the ceiling on a card with its own framebuffer, and atlasctl has no
//! business deciding that for hardware it has never seen.
//!
//! So this warns and gets out of the way. The operator keeps the decision, and
//! keeps it having been told.

/// Above this, `gpu-memory-utilization` has hung a GB10.
///
/// 0.90 froze a DGX Spark hard enough to need a power cycle. The shipped
/// flagship recipe runs at 0.88, so this sits between the two: high enough not
/// to nag anyone using a shipped recipe, low enough to speak before the region
/// that has actually cost a machine.
const GB10_UTIL_CAUTION: f64 = 0.88;

/// Whether an accelerator name is the unified-memory part this caution is about.
///
/// Matched on the family rather than an exact string: `nvidia-smi` reports
/// "NVIDIA GB10" today, and a rename or a variant should keep the warning
/// rather than silently lose it.
fn is_gb10(accelerator: &str) -> bool {
    let a = accelerator.to_ascii_lowercase();
    a.contains("gb10") || a.contains("grace blackwell")
}

/// A caution for one setting, or nothing.
///
/// `accelerator` is what the machine reports about itself; an empty string
/// means it did not say, and an unknown accelerator gets no warning — inventing
/// one for hardware we cannot identify would train people to ignore them.
#[must_use]
pub fn caution(key: &str, value: f64, accelerator: &str) -> Option<String> {
    if key != "gpu_memory_utilization" || !is_gb10(accelerator) {
        return None;
    }
    if value <= GB10_UTIL_CAUTION {
        return None;
    }
    Some(format!(
        "gpu_memory_utilization {value} is above {GB10_UTIL_CAUTION} on {accelerator}. \
         0.90 has frozen a machine of this kind hard enough to need a power cycle, and \
         unified memory leaves no framebuffer to fall back on. The shipped recipes run \
         at {GB10_UTIL_CAUTION}; raise it only if you can reach the box to reset it."
    ))
}

#[cfg(test)]
mod tests {
    use super::{caution, is_gb10};

    #[test]
    fn the_shipped_recipe_value_does_not_nag() {
        // 0.88 is what the flagship recipe ships. Warning about it would train
        // people to ignore the warning that matters.
        assert_eq!(caution("gpu_memory_utilization", 0.88, "NVIDIA GB10"), None);
        assert_eq!(caution("gpu_memory_utilization", 0.85, "NVIDIA GB10"), None);
    }

    #[test]
    fn the_value_that_froze_a_box_is_named_with_what_it_costs() {
        let c = caution("gpu_memory_utilization", 0.90, "NVIDIA GB10").expect("must warn");
        assert!(c.contains("0.9"), "{c}");
        assert!(c.contains("power cycle"), "the cost is the point: {c}");
        assert!(
            c.contains("NVIDIA GB10"),
            "name the hardware it applies to: {c}"
        );
    }

    #[test]
    fn hardware_we_cannot_identify_gets_no_invented_warning() {
        // A caution for every unknown card is a caution nobody reads.
        assert_eq!(caution("gpu_memory_utilization", 0.95, "NVIDIA H200"), None);
        assert_eq!(caution("gpu_memory_utilization", 0.95, ""), None);
    }

    #[test]
    fn only_this_setting_is_cautioned() {
        assert_eq!(caution("max_batch_size", 999.0, "NVIDIA GB10"), None);
    }

    #[test]
    fn the_family_is_matched_not_one_exact_string() {
        // A rename or a variant must keep the warning rather than lose it.
        assert!(is_gb10("NVIDIA GB10"));
        assert!(is_gb10("nvidia gb10 superchip"));
        assert!(is_gb10("NVIDIA Grace Blackwell GB10"));
        assert!(!is_gb10("NVIDIA H100 80GB HBM3"));
    }
}
