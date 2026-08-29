// SPDX-License-Identifier: AGPL-3.0-only

//! The serve flags this project deliberately does not pass through.
//!
//! `vendor/serve-options.v1.json` is reflected out of the engine's own clap
//! definition, so the set of flags is now a fact rather than a belief. Every
//! one of them must be either claimed by `ATLAS_FLAGS` or listed here with a
//! reason; a test fails otherwise. That is the whole point of vendoring the
//! snapshot — before it, 9 keys in shipping recipes were dropped on the floor
//! and nothing anywhere noticed.
//!
//! Being excluded is not the same as being unknown. A key here is a real flag
//! the engine accepts, and saying so is more useful to an operator than
//! "unmapped": it is the difference between "you made a typo" and "that is a
//! real setting, and here is why this launcher will not send it."

/// Every engine flag atlasctl will not emit, and why.
///
/// Reasons are grouped because the decisions are: the four multi-node bootstrap
/// flags are one decision, not four.
#[rustfmt::skip] // Key and reason on one line each; the pairing is the content.
pub static EXCLUDED: &[(&str, &str)] = &[
    // Placement is atlasctl's job. It derives the whole rank tuple from the
    // fleet and the chosen placement, and a recipe that also set them would be
    // fighting the launcher for control of the same four values.
    ("rank", "derived from the placement, not the recipe"),
    ("world_size", "derived from the placement, not the recipe"),
    ("master_addr", "derived from the placement, not the recipe"),
    ("master_port", "derived from the placement, not the recipe"),

    // Host paths and host processes. Same reasoning as the deny list, one step
    // earlier: these must not be settable even from a recipe file, because a
    // recipe is a downloadable artifact.
    ("kernel_target", "a host path controlling which compiled kernels load"),
    ("auth_tokens_file", "a host path holding credentials"),
    ("warmup_prompt", "a host path read at startup"),
    ("video_ffmpeg_path", "names an executable to run"),
    ("lora_adapter", "loads weights of the recipe author's choosing"),
    ("lora_stageable", "loads weights of the recipe author's choosing"),
    ("lora_stageable_disk", "loads weights of the recipe author's choosing"),
    ("max_lora_rank", "part of the LoRA family, which is excluded as a group"),
    ("max_loras", "part of the LoRA family, which is excluded as a group"),

    // Outbound network from inside the container. The engine defaults these to
    // off; a launcher that could turn them on from a recipe would be a
    // server-side request forgery primitive with a YAML front end.
    ("vision_allow_remote_images", "makes the server fetch URLs a request names"),
    ("vision_remote_image_max_mb", "only meaningful with remote images, which are excluded"),
    ("vision_remote_image_timeout_s", "only meaningful with remote images, which are excluded"),
    ("vision_remote_image_allow_private", "would let a fetch reach loopback and private ranges"),

    // Diagnostics that change what the process is. Each either exits instead of
    // serving, writes a transcript of every request, or takes the terminal.
    ("check_kernels", "resolves kernels and exits; it does not serve"),
    ("dump", "writes a transcript of every request to disk"),
    ("profile", "synchronises on every kernel; it is a measurement mode, not a serving mode"),
    ("no_tui", "atlasctl always runs the engine headless; the TUI is never on"),
    ("dangerously_allow_unresolved_kernel_lookups", "serves a model whose dispatch is known to be incomplete"),

    // Model-swapping. atlasctl owns the lifecycle of a running model — it is
    // what `atlasctl stop` and the fleet view are about — and an engine that
    // swaps models underneath it would make that view a lie.
    ("auto_swap", "atlasctl owns which model is loaded"),
    ("no_auto_swap", "atlasctl owns which model is loaded"),
    ("auto_compact", "changes conversation content server-side, which no recipe should decide"),

    // Sampling and template defaults. These belong to the request or to
    // MODEL.toml; a launcher-wide default silently changes what every client
    // gets and is invisible from the client side.
    ("default_top_n_sigma", "a per-request sampling choice"),
    ("default_min_p", "a per-request sampling choice"),
    ("adaptive_sampling", "a per-request sampling choice"),
    ("default_chat_template_kwargs", "a per-request templating choice"),
    ("disable_template_overrides", "MODEL.toml's business, not the launcher's"),
    ("max_inter_tool_prose", "MODEL.toml's business, not the launcher's"),
    ("content_loop_watchdog", "MODEL.toml's business, not the launcher's"),
    ("content_loop_min_repeats", "MODEL.toml's business, not the launcher's"),
    ("src_lang", "NLLB/M2M-100 only; no recipe here serves one"),
    ("tgt_lang", "NLLB/M2M-100 only; no recipe here serves one"),

    // Marked EXPERIMENTAL or OPT-IN in the engine's own help text. Passing
    // these through would make this launcher the place they get exercised.
    ("ssm_rollback_mode", "the engine marks it EXPERIMENTAL"),
    ("exact_verify", "the engine marks it OPT-IN"),
    ("high_speed_swap_graph", "the engine marks it a phased rollout"),

    // Loader and memory knobs with no recipe asking for them. Not a judgement
    // that they are wrong — nothing has needed them, and an unexercised
    // passthrough is an untested one.
    ("no_fast_load", "no recipe needs it; add it when one does"),
    ("fast_load_prefetch_shards", "no recipe needs it; add it when one does"),
    ("swap_space_gb", "no recipe needs it; add it when one does"),
    ("fp8_kv_headroom", "no recipe needs it; add it when one does"),
    ("vision_max_pixels", "no recipe needs it; add it when one does"),
    ("video_fps", "no recipe needs it; add it when one does"),
    ("video_max_frames", "no recipe needs it; add it when one does"),
    ("video_decode_timeout_s", "no recipe needs it; add it when one does"),
];

/// Why a key the engine accepts is not emitted, if that is what it is.
///
/// Returns `None` for a key that is claimed, and for one the engine has never
/// heard of — those are different failures, and the caller distinguishes them
/// by first asking `flags::lookup`.
pub fn excluded_reason(key: &str) -> Option<&'static str> {
    EXCLUDED.iter().find(|(k, _)| *k == key).map(|(_, r)| *r)
}

#[cfg(test)]
mod tests;
