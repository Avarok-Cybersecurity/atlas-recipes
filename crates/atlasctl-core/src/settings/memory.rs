// SPDX-License-Identifier: AGPL-3.0-only

//! The flags a client may set, and how — memory, KV cache and speculation.
//!
//! A continuation of `table.rs`, not a separate category: the two are
//! concatenated in declaration order, because that order is what a form
//! renders. They are two files because one would be over this project's
//! per-file limit, and splitting at a group boundary is the split that costs a
//! reader nothing.

use super::spec::{BoundSpec, Disposition, Spec};
use super::values::{DTYPES, KV_DTYPES, LM_HEAD_DTYPES, MTP_GATES, SSM_H_DTYPES};
use atlasctl_protocol::settings::Group;

use BoundSpec::{BoolValue, Enum, Float, Int, IntOrAuto, Toggle};
use Disposition::Expose;
use Group::{MemoryKv, Performance, Server, Speculative, ToolsChat};

/// Terse constructor, so the table reads as a table.
const fn e(
    bound: BoundSpec,
    label: &'static str,
    help: &'static str,
    unit: Option<&'static str>,
    group: Group,
    advanced: bool,
) -> Disposition {
    Expose(Spec {
        bound,
        label,
        help,
        unit,
        group,
        advanced,
    })
}

/// The second half of the exposed table. See `table::EXPOSED`.
#[rustfmt::skip] // One line per flag: this is a lookup table.
pub static EXPOSED_MEMORY: &[(&str, Disposition)] = &[
    // --- Memory & KV --------------------------------------------------------
    (
        "gpu_memory_utilization",
        e(
            Float(0.10, 0.95),
            "Memory fraction",
            "Share of the memory pool the engine may use.",
            None,
            MemoryKv,
            false,
        ),
    ),
    (
        "max_model_len",
        e(
            Int(256, 262144),
            "Context length",
            "Longest sequence the server will accept.",
            Some("tokens"),
            MemoryKv,
            false,
        ),
    ),
    (
        "kv_cache_dtype",
        e(
            Enum(KV_DTYPES),
            "KV cache dtype",
            "Precision of the key/value cache.",
            None,
            MemoryKv,
            false,
        ),
    ),
    (
        "kv_high_precision_layers",
        e(
            IntOrAuto(0, 256),
            "High-precision KV layers",
            "Layers kept at higher KV precision.",
            None,
            MemoryKv,
            true,
        ),
    ),
    (
        "block_size",
        e(
            Enum(&["8", "16", "32", "64"]),
            "KV block size",
            "Paged-attention block size.",
            Some("tokens"),
            MemoryKv,
            true,
        ),
    ),
    (
        "oom_guard_mb",
        e(
            Int(0, 16384),
            "OOM guard",
            "Memory held back as headroom.",
            Some("MB"),
            MemoryKv,
            true,
        ),
    ),
    (
        "fp8_kv_calibration_tokens",
        e(
            Int(0, 65536),
            "FP8 KV calibration",
            "Tokens used to calibrate an FP8 KV cache.",
            Some("tokens"),
            MemoryKv,
            true,
        ),
    ),
    (
        "ssm_cache_slots",
        e(
            Int(0, 4096),
            "SSM cache slots",
            "State-space cache slots preallocated.",
            None,
            MemoryKv,
            true,
        ),
    ),
    (
        "ssm_checkpoint_interval",
        e(
            Int(1, 8192),
            "SSM checkpoint interval",
            "How often SSM state is checkpointed.",
            None,
            MemoryKv,
            true,
        ),
    ),
    // --- Speculative decoding -----------------------------------------------
    (
        "speculative",
        e(
            Toggle,
            "Speculative decoding",
            "Draft tokens ahead and verify them.",
            None,
            Speculative,
            false,
        ),
    ),
    (
        "self_speculative",
        e(
            Toggle,
            "Self-speculative",
            "Draft with the model's own early layers.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "ngram_speculative",
        e(
            Toggle,
            "N-gram speculative",
            "Draft from prompt n-grams.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "dflash",
        e(
            Toggle,
            "DFlash",
            "Enable the DFlash draft path.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "num_drafts",
        e(
            Int(1, 8),
            "Draft tokens",
            "Tokens drafted per verification step.",
            None,
            Speculative,
            false,
        ),
    ),
    (
        "mtp_quantization",
        e(
            Enum(DTYPES),
            "MTP precision",
            "Precision of the draft head.",
            None,
            Speculative,
            false,
        ),
    ),
    (
        "dflash_gamma",
        e(
            Int(1, 16),
            "DFlash gamma",
            "DFlash draft depth.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "dflash_window_size",
        e(
            Int(1, 8192),
            "DFlash window",
            "DFlash lookahead window.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "tool_max_tokens",
        e(
            Int(64, 65536),
            "Tool call budget",
            "Token budget for a tool call.",
            Some("tokens"),
            ToolsChat,
            true,
        ),
    ),
    (
        "disable_tool_grammar",
        e(
            BoolValue,
            "Disable tool grammar",
            "Turn off grammar-constrained tool calls.",
            None,
            ToolsChat,
            true,
        ),
    ),
    (
        "disable_thinking",
        e(
            Toggle,
            "Disable thinking",
            "Suppress reasoning output.",
            None,
            ToolsChat,
            false,
        ),
    ),
    (
        "max_thinking_budget",
        e(
            Int(0, 262144),
            "Thinking budget",
            "Cap on reasoning tokens.",
            Some("tokens"),
            ToolsChat,
            true,
        ),
    ),
    // --- Claimed 2026-08-26 -------------------------------------------------
    // Set by shipping recipes and dropped on the floor until the engine
    // snapshot made the omission visible. Bounds are this project's own: the
    // engine's clap definition carries value sets but no ranges at all.
    (
        "lm_head_dtype",
        e(
            Enum(LM_HEAD_DTYPES),
            "LM head precision",
            "Precision of the output projection. Recipes pin this for correctness, not speed.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "ssm_h_dtype",
        e(
            Enum(SSM_H_DTYPES),
            "SSM state precision",
            "Precision of the recurrent state carried between chunks.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "gdn_fused_norm",
        e(
            BoolValue,
            "Fused GDN norm",
            "Fuse the gated-delta-net normalisation into the preceding kernel.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "ssm_batched_recurrent",
        e(
            BoolValue,
            "Batched recurrence",
            "Run the recurrent tail batched across sequences rather than one at a time.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "ssm_tail_midchunk",
        e(
            BoolValue,
            "Mid-chunk tail",
            "Start the recurrent tail inside a chunk instead of at its boundary.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "prefill_varlen_batch",
        e(
            BoolValue,
            "Variable-length prefill",
            "Batch prefills of differing lengths without padding them to match.",
            None,
            Performance,
            true,
        ),
    ),
    (
        "mtp_gate",
        e(
            Enum(MTP_GATES),
            "MTP gate",
            "Whether multi-token prediction is used when available, or always.",
            None,
            Speculative,
            true,
        ),
    ),
    (
        "request_timeout",
        e(
            Int(0, 86_400),
            "Request timeout",
            "How long a single request may run before it is cut. 0 disables the deadline.",
            Some("s"),
            Server,
            true,
        ),
    ),
];
