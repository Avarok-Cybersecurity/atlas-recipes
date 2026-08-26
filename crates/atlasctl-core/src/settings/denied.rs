// SPDX-License-Identifier: AGPL-3.0-only

//! The flags no client may set, each with the reason.
//!
//! This is the security boundary for remote launches, and it is deliberately
//! hand-written. Nothing here can be derived from the engine: the engine has no
//! opinion about which of its flags are safe to expose to a web page, and a
//! generator that guessed would guess wrong in the expensive direction.
//!
//! A denied key is never offered in `schema()`, so an attempt to set one is a
//! signal rather than a typo — `settings::validate` reports it as `Denied` with
//! the reason, and the caller logs it.

use super::spec::Disposition;

use Disposition::Deny;

/// Every flag a client may not set, and why.
#[rustfmt::skip] // Reason strings are the content; wrapping them hides it.
pub static DENIED: &[(&str, Disposition)] = &[
    (
        "host",
        Deny("the bind address is a network-exposure decision the agent owns, not a client"),
    ),
    (
        "served_model_name",
        Deny("an unbounded name with no operational value; the only free string in the table"),
    ),
    (
        "mtp_vocab",
        Deny("couples to the checkpoint's draft-head artifact; changing it changes what loads"),
    ),
    (
        "draft_model",
        Deny("loads a second set of weights of the sender's choosing"),
    ),
    (
        "tool_call_parser",
        Deny(
            "model-coupled correctness pin; changing it away from the recipe can only break tool calls",
        ),
    ),
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
    (
        "video_allow_ffmpeg",
        Deny("spawns an external decoder process; a recipe may enable that, a client may not"),
    ),
];
