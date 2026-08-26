// SPDX-License-Identifier: AGPL-3.0-only

//! The enumerated value sets, transcribed from the engine.
//!
//! Separate from the table because they answer to a different authority.
//! The table is this project's decision about what a web page may set; these
//! are facts about a serving runtime in another repo, and they are the ones
//! that go stale silently — a bound that drifts is caught by a bijection
//! test, a VALUE that drifts is caught by a launch dying in a container.
//!
//! Every list here has a matching assertion in `settings/tests.rs`, naming
//! the engine symbol it came from.

/// Every KV-cache precision the serving runtime accepts.
///
/// Transcribed from `spark_runtime::kv_cache::KvCacheDtype::ALL`, which is
/// derived from the enum under a wildcard-free match so adding a variant
/// fails that build rather than silently missing from a picker. This list is
/// the one place that transcription lives; `settings/tests.rs` records the
/// count so a drift shows up as a failing test rather than as a launch that
/// dies inside the container.
///
/// The short aliases the CLI also accepts (`fp8k2v` for `fp8k_turbo2v`) are
/// deliberately not offered: a picker should name one spelling.
pub(super) const KV_DTYPES: &[&str] = &[
    "bf16",
    "fp8",
    "nvfp4",
    "turbo4",
    "turbo3",
    "turbo2",
    "turbo8",
    "turbo4k_turbo3v",
    "turbo4k_turbo8v",
    "turbo3k_turbo8v",
    "bf16k_turbo4v",
    "bf16k_turbo3v",
    "fp8k_turbo4v",
    "fp8k_turbo3v",
    "bf16k_turbo2v",
    "fp8k_turbo2v",
];

/// Precisions for weights-adjacent settings, which are not the KV set.
pub(super) const DTYPES: &[&str] = &["bf16", "fp8", "nvfp4"];

/// Precisions the output projection accepts.
///
/// Transcribed from the engine's clap value set for `--lm-head-dtype`, which
/// carries a `default` variant the weights-adjacent set does not.
pub(super) const LM_HEAD_DTYPES: &[&str] = &["default", "bf16", "nvfp4", "fp8"];

/// Precisions for the recurrent state carried between chunks.
///
/// `f16-pool` is the odd spelling in the engine's set: hyphen, not underscore.
pub(super) const SSM_H_DTYPES: &[&str] = &["f32", "f16", "f16-pool"];

/// When multi-token prediction runs.
pub(super) const MTP_GATES: &[&str] = &["auto", "force"];
