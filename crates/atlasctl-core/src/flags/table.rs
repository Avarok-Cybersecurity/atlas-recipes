// SPDX-License-Identifier: AGPL-3.0-only

//! The `spark serve` flag table.
//!
//! Generated from the reference implementation's `_ATLAS_FLAG_MAP` and
//! `_ATLAS_BOOL_FLAGS` (sparkrun `runtimes/atlas.py`, lines 43-126) and kept in
//! that exact declaration order, because emission order is part of the output
//! contract: the website prints the same command string this table renders, and
//! the golden corpus asserts it byte-for-byte.
//!
//! Two keys deliberately map to the same flag (`max_num_batched_tokens` and
//! `max_prefill_tokens` both render `--max-prefill-tokens`). The reference
//! silently emitted the flag twice if both were set; we reject that as a
//! validation error instead — see `flags::validate_no_alias_conflict`.

use super::{FlagKind, FlagSpec};

/// Every serve flag the `atlas` runtime understands: 48 keys, 8 of them bare toggles.
#[rustfmt::skip] // One line per flag: this is a lookup table, and reading it
// against the reference implementation is the point.
pub static ATLAS_FLAGS: [FlagSpec; 48] = [
    FlagSpec { key: "port", flag: "--port", kind: FlagKind::Value },
    FlagSpec { key: "host", flag: "--host", kind: FlagKind::Value },
    FlagSpec { key: "tensor_parallel", flag: "--tp-size", kind: FlagKind::Value },
    FlagSpec { key: "gpu_memory_utilization", flag: "--gpu-memory-utilization", kind: FlagKind::Value },
    FlagSpec { key: "max_model_len", flag: "--max-seq-len", kind: FlagKind::Value },
    FlagSpec { key: "max_num_seqs", flag: "--max-num-seqs", kind: FlagKind::Value },
    FlagSpec { key: "max_num_batched_tokens", flag: "--max-prefill-tokens", kind: FlagKind::Value },
    FlagSpec { key: "served_model_name", flag: "--model-name", kind: FlagKind::Value },
    FlagSpec { key: "kv_cache_dtype", flag: "--kv-cache-dtype", kind: FlagKind::Value },
    FlagSpec { key: "ep_size", flag: "--ep-size", kind: FlagKind::Value },
    FlagSpec { key: "max_batch_size", flag: "--max-batch-size", kind: FlagKind::Value },
    FlagSpec { key: "block_size", flag: "--block-size", kind: FlagKind::Value },
    FlagSpec { key: "kv_high_precision_layers", flag: "--kv-high-precision-layers", kind: FlagKind::Value },
    FlagSpec { key: "tool_call_parser", flag: "--tool-call-parser", kind: FlagKind::Value },
    FlagSpec { key: "tool_max_tokens", flag: "--tool-max-tokens", kind: FlagKind::Value },
    FlagSpec { key: "disable_tool_grammar", flag: "--disable-tool-grammar", kind: FlagKind::Value },
    FlagSpec { key: "fp8_kv_calibration_tokens", flag: "--fp8-kv-calibration-tokens", kind: FlagKind::Value },
    FlagSpec { key: "scheduling_policy", flag: "--scheduling-policy", kind: FlagKind::Value },
    FlagSpec { key: "tbt_deadline_ms", flag: "--tbt-deadline-ms", kind: FlagKind::Value },
    FlagSpec { key: "max_prefill_tokens", flag: "--max-prefill-tokens", kind: FlagKind::Value },
    FlagSpec { key: "oom_guard_mb", flag: "--oom-guard-mb", kind: FlagKind::Value },
    FlagSpec { key: "ssm_cache_slots", flag: "--ssm-cache-slots", kind: FlagKind::Value },
    FlagSpec { key: "ssm_checkpoint_interval", flag: "--ssm-checkpoint-interval", kind: FlagKind::Value },
    FlagSpec { key: "mtp_quantization", flag: "--mtp-quantization", kind: FlagKind::Value },
    FlagSpec { key: "mtp_vocab", flag: "--mtp-vocab", kind: FlagKind::Value },
    FlagSpec { key: "num_drafts", flag: "--num-drafts", kind: FlagKind::Value },
    FlagSpec { key: "draft_model", flag: "--draft-model", kind: FlagKind::Value },
    FlagSpec { key: "dflash_gamma", flag: "--dflash-gamma", kind: FlagKind::Value },
    FlagSpec { key: "dflash_window_size", flag: "--dflash-window-size", kind: FlagKind::Value },
    FlagSpec { key: "max_thinking_budget", flag: "--max-thinking-budget", kind: FlagKind::Value },
    FlagSpec { key: "model_from_path", flag: "--model-from-path", kind: FlagKind::Value },
    FlagSpec { key: "cache_dir", flag: "--cache-dir", kind: FlagKind::Value },
    FlagSpec { key: "gpu_ordinal", flag: "--gpu-ordinal", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_dir", flag: "--high-speed-swap-dir", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_gb", flag: "--high-speed-swap-gb", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_resident_blocks", flag: "--high-speed-swap-resident-blocks", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_rank", flag: "--high-speed-swap-rank", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_qd", flag: "--high-speed-swap-qd", kind: FlagKind::Value },
    FlagSpec { key: "high_speed_swap_cache_blocks_per_seq", flag: "--high-speed-swap-cache-blocks-per-seq", kind: FlagKind::Value },
    FlagSpec { key: "enable_prefix_caching", flag: "--enable-prefix-caching", kind: FlagKind::BoolToggle },
    FlagSpec { key: "speculative", flag: "--speculative", kind: FlagKind::BoolToggle },
    FlagSpec { key: "self_speculative", flag: "--self-speculative", kind: FlagKind::BoolToggle },
    FlagSpec { key: "ngram_speculative", flag: "--ngram-speculative", kind: FlagKind::BoolToggle },
    FlagSpec { key: "dflash", flag: "--dflash", kind: FlagKind::BoolToggle },
    FlagSpec { key: "disable_thinking", flag: "--disable-thinking", kind: FlagKind::BoolToggle },
    FlagSpec { key: "high_speed_swap", flag: "--high-speed-swap", kind: FlagKind::BoolToggle },
    FlagSpec { key: "require_auth", flag: "--require-auth", kind: FlagKind::BoolToggle },
    FlagSpec { key: "api_key", flag: "--auth-token", kind: FlagKind::Value },
];
