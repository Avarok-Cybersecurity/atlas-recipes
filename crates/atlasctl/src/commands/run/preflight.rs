// SPDX-License-Identifier: AGPL-3.0-only

//! What to say when a launch cannot start, or did not survive.
//!
//! Split from `run.rs` for the 500-line cap, along a seam that was already
//! there: everything here answers "why can this not run?" rather than running
//! it, and every message is built and asserted rather than formatted inline —
//! these are read at a moment of failure, so their wording is the feature.

use atlasctl_core::hfcache;

/// How long to give a container to prove it did not die on the spot.
const LIVENESS_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// `Some(explanation)` when the container is already gone.
pub(super) fn died_immediately(
    runner: &dyn atlasctl_core::io::ProcessRunner,
    recipe: &str,
    container: &str,
) -> Option<String> {
    std::thread::sleep(LIVENESS_WAIT);
    let out = runner.run(&liveness_argv(recipe)).ok();
    liveness_verdict(out.as_ref(), container, recipe)
}

/// Ask docker about THIS recipe's container, by label.
///
/// Not `--filter name=`: docker matches that as a substring, and recipe names
/// nest — `qwen3.6-27b-fp8` is inside `qwen3.6-27b-fp8-mtp`, and three more
/// pairs do the same. A name filter would see the sibling still running and
/// report the container that just died as alive, which is silence in exactly
/// the case this check exists for.
///
/// The label is what the launch writes and what `atlasctl logs`/`stop` already
/// match on, so all three agree about which container belongs to a recipe.
pub(super) fn liveness_argv(recipe: &str) -> Vec<String> {
    vec![
        "docker".to_string(),
        "ps".to_string(),
        "-q".to_string(),
        "--filter".to_string(),
        format!(
            "label={}={recipe}",
            atlasctl_core::docker::translate::LABEL_RECIPE
        ),
    ]
}

/// The judgement, separated from the waiting and the process call so it can be
/// tested without either.
///
/// `None` covers both "still running" and "cannot tell". A docker that will not
/// answer `ps` is not evidence the launch failed, and refusing a launch that
/// worked because a status query hiccuped is worse than the silence this
/// replaces.
pub(super) fn liveness_verdict(
    out: Option<&atlasctl_core::io::process::Output>,
    name: &str,
    recipe_arg: &str,
) -> Option<String> {
    let out = out?;
    if !out.success() || !out.stdout.trim().is_empty() {
        return None;
    }
    Some(format!(
        concat!(
            "`{name}` started and then exited within {secs}s.\n",
            "That is the container failing at load rather than the launch failing: ",
            "docker accepted it, and the engine stopped afterwards.\n",
            "Its logs are already gone, because the launch runs with `--rm`. ",
            "Re-run keeping the container so they survive:\n",
            "    atlasctl run {name_arg} --no-rm\n",
            "then read them with `atlasctl logs {name_arg}`. The usual causes are ",
            "an image with no kernel target for this checkpoint, a KV dtype the ",
            "engine refuses, and not enough memory.",
        ),
        name = name,
        secs = LIVENESS_WAIT.as_secs(),
        name_arg = recipe_arg
    ))
}

#[cfg(test)]
mod liveness_tests {
    use super::liveness_verdict;
    use atlasctl_core::io::process::Output;

    fn out(status: i32, stdout: &str) -> Output {
        Output {
            status,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    /// An empty `docker ps` means the container is gone, and the message has to
    /// say the two things the operator cannot work out alone: that docker
    /// accepted the launch, and that `atlasctl logs` will not help because the
    /// container was removed on exit.
    #[test]
    fn an_empty_ps_is_a_container_that_died() {
        let why = liveness_verdict(Some(&out(0, "")), "atlas-r", "r").expect("gone");
        assert!(why.contains("atlas-r"), "must name it: {why}");
        assert!(why.contains("--rm"), "must explain the missing logs: {why}");
        assert!(
            why.contains("--no-rm"),
            "must offer a way to actually see the failure: {why}"
        );
        assert!(
            why.contains("atlasctl logs r"),
            "and the command that then reads them: {why}"
        );
    }

    /// A container id means it is still running: say nothing.
    #[test]
    fn a_running_container_is_not_reported() {
        assert!(liveness_verdict(Some(&out(0, "9f3c1a2b\n")), "atlas-r", "r").is_none());
    }

    /// The query must be by LABEL, not by name.
    ///
    /// Docker matches `--filter name=` as a substring, and recipe names nest:
    /// `qwen3.6-27b-fp8` is inside `qwen3.6-27b-fp8-mtp`, and three other pairs
    /// in the shipped corpus do the same. With a name filter, launching the
    /// shorter recipe while the longer one runs would match the SIBLING and
    /// report the dead container as alive — silence in the one case this check
    /// exists for.
    #[test]
    fn the_query_is_by_label_so_a_sibling_recipe_cannot_answer_for_it() {
        let argv = super::liveness_argv("qwen3.6-27b-fp8").join(" ");
        assert!(
            argv.contains("label=io.atlasctl.recipe=qwen3.6-27b-fp8"),
            "must ask by label: {argv}"
        );
        assert!(
            !argv.contains("name="),
            "a name filter is a substring match and would match the -mtp sibling: {argv}"
        );
    }

    /// A docker that will not answer is NOT evidence of failure. Refusing a
    /// launch that worked, because a status query hiccuped, would be a worse
    /// bug than the silence this check replaces.
    #[test]
    fn an_unanswerable_docker_is_not_treated_as_a_dead_container() {
        assert!(
            liveness_verdict(Some(&out(1, "")), "atlas-r", "r").is_none(),
            "a failed ps must not be read as a dead container"
        );
        assert!(
            liveness_verdict(None, "atlas-r", "r").is_none(),
            "a runner error must not be read as a dead container"
        );
    }
}

/// What to say when the model a recipe needs is not usable from the cache.
///
/// `None` when it is. Built here rather than inline so the WORDING can be
/// tested: these are read at a moment of failure, and the useful half is the
/// command at the end, which has to survive both a refactor and rustfmt.
pub(super) fn cache_miss(
    state: hfcache::CacheState,
    recipe_arg: &str,
    model: &str,
    cache_dir: &str,
) -> Option<String> {
    match state {
        hfcache::CacheState::Weights => None,
        hfcache::CacheState::Absent => Some(format!(
            concat!(
                "`{recipe}` needs the model `{model}`, which is not in {dir}.\n",
                "The launch runs offline, so it cannot download it. Fetch it first:\n",
                "    hf download {model}",
            ),
            recipe = recipe_arg,
            model = model,
            dir = cache_dir
        )),
        // Distinct from Absent on purpose. `hf download` is the same fix, but
        // "it is not there" is the wrong thing to tell someone who can SEE the
        // directory: they check, find it, and conclude the tool is broken.
        // Naming the state — present, but no weights — is what makes the
        // instruction believable.
        hfcache::CacheState::MetadataOnly => Some(format!(
            concat!(
                "`{recipe}` needs the model `{model}`. Its cache directory exists in ",
                "{dir}, but holds no weight files — only metadata, which is what an ",
                "interrupted or metadata-only download leaves behind.\n",
                "The launch runs offline, so it cannot fetch the rest. Complete it with:\n",
                "    hf download {model}",
            ),
            recipe = recipe_arg,
            model = model,
            dir = cache_dir
        )),
    }
}

#[cfg(test)]
mod cache_miss_tests {
    use super::cache_miss;
    use atlasctl_core::hfcache::CacheState;

    /// Usable weights say nothing at all.
    #[test]
    fn a_present_model_is_not_reported() {
        assert!(cache_miss(CacheState::Weights, "r", "org/m", "/c").is_none());
    }

    /// Both failures end in a command the operator can run, indented so it is
    /// findable in a wall of prose.
    ///
    /// The indent is asserted because it did NOT survive the first version:
    /// written as a `\`-continued string, Rust strips the following line's
    /// leading whitespace, so `hf download` rendered flush against the margin
    /// and stopped looking like a command.
    #[test]
    fn both_failures_end_in_an_indented_command() {
        for state in [CacheState::Absent, CacheState::MetadataOnly] {
            let why = cache_miss(state, "r", "org/m", "/c").expect("must explain");
            assert!(
                why.contains("\n    hf download org/m"),
                "the command must be indented and name the model: {why:?}"
            );
        }
    }

    /// The two states must not converge on one message: an operator who can see
    /// the directory is told the model is missing, decides the tool is wrong,
    /// and stops believing the fix it just offered.
    #[test]
    fn a_metadata_only_cache_is_not_described_as_absent() {
        let absent = cache_miss(CacheState::Absent, "r", "org/m", "/c").expect("absent");
        let meta = cache_miss(CacheState::MetadataOnly, "r", "org/m", "/c").expect("meta");
        assert!(absent.contains("not in /c"), "{absent}");
        assert!(meta.contains("no weight files"), "{meta}");
        assert!(
            !meta.contains("which is not in"),
            "a directory the operator can see must not be called absent: {meta}"
        );
    }
}
