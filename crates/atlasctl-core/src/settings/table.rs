// SPDX-License-Identifier: AGPL-3.0-only

//! The allow/deny partition of every serve flag.
//!
//! This table is the security boundary for remote launches: a key that is not
//! here cannot be set, and a key marked `Deny` cannot be set by a client no
//! matter what it sends. It must stay exhaustive over the flag table, which a
//! test enforces.

use super::spec::{BoundSpec, Disposition, Spec};
use super::values::{DTYPES, KV_DTYPES};
use atlasctl_protocol::settings::Group;

use BoundSpec::{BoolValue, Enum, Float, Int, IntOrAuto, Toggle};
use Disposition::{Deny, Expose};
use Group::{MemoryKv, Performance, Server, Speculative, ToolsChat, Topology};

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

/// Every flag, and whether a client may set it.
///
/// This must stay exhaustive over the flag table: a test asserts the two are in
/// bijection, so adding flag 49 without deciding whether a webpage may set it
/// fails the build rather than defaulting to either answer.
///
/// Formatting is frozen at one line per flag: reading this against the flag
/// table is how the partition gets checked by eye, and a formatter that
/// explodes each entry across six lines makes that impossible.
#[rustfmt::skip]
pub static DISPOSITIONS: &[(&str, Disposition)] = &[
    // --- Server -------------------------------------------------------------
    (
        "port",
        e(
            Int(1024, 49151),
            "Port",
            "Port the model server listens on.",
            None,
            Server,
            false,
        ),
    ),
    (
        "host",
        Deny("the bind address is a network-exposure decision the agent owns, not a client"),
    ),
    (
        "served_model_name",
        Deny("an unbounded name with no operational value; the only free string in the table"),
    ),
    // --- Topology (recipe-owned) --------------------------------------------
    (
        "tensor_parallel",
        e(
            Int(1, 8),
            "Tensor parallel",
            "Tensor-parallel degree.",
            None,
            Topology,
            true,
        ),
    ),
    (
        "ep_size",
        e(
            Int(1, 8),
            "Expert parallel",
            "Expert-parallel degree.",
            None,
            Topology,
            true,
        ),
    ),
    // --- Performance --------------------------------------------------------
    (
        "max_batch_size",
        e(
            Int(1, 64),
            "Max batch size",
            "Concurrent sequences decoded together.",
            None,
            Performance,
            false,
        ),
    ),
    (
        "max_num_seqs",
        e(
            Int(1, 256),
            "Max sequences",
            "Concurrent sequences admitted.",
            None,
            Performance,
            false,
        ),
    ),
    (
        "max_prefill_tokens",
        e(
            Int(256, 262144),
            "Max prefill tokens",
            "Largest prefill chunk.",
            Some("tokens"),
            Performance,
            false,
        ),
    ),
    (
        "max_num_batched_tokens",
        e(
            Int(256, 262144),
            "Max batched tokens",
            "Alias of max prefill tokens.",
            Some("tokens"),
            Performance,
            true,
        ),
    ),
    (
        "scheduling_policy",
        e(
            // "fifo", not "fcfs". The engine validates against
            // `cli::flag_values::SCHEDULING_POLICIES`, which has never
            // contained "fcfs" — offering it produced a launch that died in
            // validate_serve_args, on the machine, after the operator had
            // already reviewed the command.
            Enum(&["fifo", "slai"]),
            "Scheduling policy",
            "How queued requests are ordered.",
            None,
            Performance,
            false,
        ),
    ),
    (
        "tbt_deadline_ms",
        e(
            Int(1, 10000),
            "Time-between-tokens deadline",
            "Latency target the scheduler aims for.",
            Some("ms"),
            Performance,
            true,
        ),
    ),
    (
        "enable_prefix_caching",
        e(
            Toggle,
            "Prefix caching",
            "Reuse KV for shared prompt prefixes.",
            None,
            Performance,
            false,
        ),
    ),
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
        "mtp_vocab",
        Deny("couples to the checkpoint's draft-head artifact; changing it changes what loads"),
    ),
    (
        "draft_model",
        Deny("loads a second set of weights of the sender's choosing"),
    ),
    // --- Tools & chat -------------------------------------------------------
    (
        "tool_call_parser",
        Deny(
            "model-coupled correctness pin; changing it away from the recipe can only break tool calls",
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
    // --- Denied: paths, credentials, and host hardware ----------------------
    (
        "model_from_path",
        Deny(
            "a filesystem path that changes which weights load — the exact vector this project exists to close",
        ),
    ),
    (
        "cache_dir",
        Deny("a host path controlling where weights are read and written"),
    ),
    (
        "gpu_ordinal",
        Deny("host hardware selection, not a property of the deployment"),
    ),
    (
        "api_key",
        Deny(
            "a credential must not travel from a page into a command line, where `docker inspect` can read it",
        ),
    ),
    (
        "require_auth",
        Deny("letting a client turn authentication off is a downgrade attack"),
    ),
    (
        "high_speed_swap",
        Deny("unusable without a swap directory, which is a host path"),
    ),
    (
        "high_speed_swap_dir",
        Deny("a host path the engine writes to"),
    ),
    (
        "high_speed_swap_gb",
        Deny("part of the swap family, which is denied as a group"),
    ),
    (
        "high_speed_swap_resident_blocks",
        Deny("part of the swap family, which is denied as a group"),
    ),
    (
        "high_speed_swap_rank",
        Deny("part of the swap family, which is denied as a group"),
    ),
    (
        "high_speed_swap_qd",
        Deny("part of the swap family, which is denied as a group"),
    ),
    (
        "high_speed_swap_cache_blocks_per_seq",
        Deny("part of the swap family, which is denied as a group"),
    ),
];
