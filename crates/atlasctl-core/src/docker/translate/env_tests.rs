// SPDX-License-Identifier: AGPL-3.0-only

//! What a recipe may read from the host, and what it may not set.
//!
//! Split from `tests.rs` on the 500-line cap, and the seam is real: that file
//! is about turning a recipe into a command, and these are about the trust
//! boundary around the recipe itself. Recipes arrive from a remote index and
//! name the image they run, so "what can this content reach" is its own
//! question with its own answers.

use super::tests::{ctx, host, plan, recipe};
use crate::chain::{Overrides, UserConfig};
use crate::docker::Placement;
use crate::docker::profile::NvidiaDevices;
use crate::docker::translate::{TranslateError, translate};
use crate::recipe::Recipe;

/// The refusing half of `plan`, here because this is the only file that needs
/// a translate that fails.
fn plan_err(r: &Recipe, p: &Placement) -> TranslateError {
    translate(
        r,
        &Overrides::new(),
        &UserConfig::new(),
        &host(),
        p,
        &ctx(&NvidiaDevices),
    )
    .expect_err("must refuse")
}

/// The half of "the recipe is the more specific intent" that survives: a
/// container-internal path is the recipe's business.
#[test]
fn a_recipe_still_overrides_a_container_internal_path() {
    let p = plan(&recipe("env:\n  HF_HOME: /custom\n"), &Placement::Solo);
    assert_eq!(p.docker.env["HF_HOME"], "/custom");
}

/// The half that does not. A recipe is remote, third-party-extensible content
/// and it also names the image it runs, so expanding `$VAR` against the
/// agent's whole environment let it read a credential and hand it to code of
/// its own choosing. This test used to assert the opposite, with `$TOKEN` as
/// its example — which reads less like a considered trade-off than like the
/// hazard never coming up.
#[test]
fn a_recipe_cannot_read_a_host_variable_outside_the_allowlist() {
    let p = plan(&recipe("env:\n  KEY: \"pre-$TOKEN\"\n"), &Placement::Solo);
    assert_eq!(
        p.docker.env["KEY"], "pre-",
        "a name outside the allowlist must read as unset, not as the host's value"
    );
}

/// The allowlist is not empty: proxy settings are the one class of host
/// variable a container legitimately needs.
#[test]
fn a_proxy_variable_still_expands() {
    let p = plan(
        &recipe("env:\n  KEY: \"via-$HTTPS_PROXY\"\n"),
        &Placement::Solo,
    );
    assert_eq!(p.docker.env["KEY"], "via-http://proxy:8080");
}

/// Refused, not ignored. A recipe setting this is affirmatively trying to
/// change the operator's egress policy; dropping it silently would launch
/// under a policy its author did not choose.
#[test]
fn a_recipe_that_sets_the_offline_switch_is_refused_by_name() {
    let err = plan_err(&recipe("env:\n  HF_HUB_OFFLINE: \"0\"\n"), &Placement::Solo);
    let text = format!("{err}");
    assert!(text.contains("HF_HUB_OFFLINE"), "must name the key: {text}");
}
