// SPDX-License-Identifier: AGPL-3.0-only

//! The flags a client may set, and how — the shape of the machine.
//!
//! Three files hold the partition, and the split is not filing.
//! `denied.rs` contributes nothing to `schema()`, so it is read by a different
//! audience: this is a UI description, that is a list of refusals with reasons.
//! `memory.rs` continues this one, and the two are concatenated in declaration
//! order because that order is what a form renders. Together the three must
//! stay exhaustive over the flag table, which a test enforces.

use super::spec::{BoundSpec, Disposition, Spec};

use atlasctl_protocol::settings::Group;

use BoundSpec::{Enum, Int, Toggle};
use Disposition::Expose;
use Group::{Performance, Server, Topology};

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

/// Server, topology and performance flags, in the order a form should show them.
///
/// This, `memory::EXPOSED_MEMORY` and `denied::DENIED` must together stay
/// exhaustive over the flag table: a test asserts they are in bijection with
/// it, so adding a flag without deciding whether a webpage may set it fails the
/// build rather than defaulting to either answer.
///
/// Formatting is frozen at one line per flag: reading this against the flag
/// table is how the partition gets checked by eye, and a formatter that
/// explodes each entry across six lines makes that impossible.
#[rustfmt::skip]
pub static EXPOSED: &[(&str, Disposition)] = &[
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
            // Matches `max_num_seqs`, which bounds the same concurrency
            // dimension. It was 1..=64 until 2026-08-26, which rejected the
            // 128 that `qwen3.8-27b-nvfp4-throughput` ships.
            Int(1, 256),
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
];
