// SPDX-License-Identifier: AGPL-3.0-only

//! `logs`' container lookup, split out when `tests.rs` reached the 500-line cap
//! — the same reason `tests.rs` itself was split out of `lifecycle.rs`.
//!
//! Split verbatim: the module body is unchanged, only its declaration moved.

/// `logs` used to guess the container name, so it could not find the two
/// launches whose names it does not spell.
mod logs_finds_what_run_started {
    use super::super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    fn args(recipe: &str) -> LogsArgs {
        LogsArgs {
            recipe: recipe.to_owned(),
            follow: false,
            tail: 10,
        }
    }

    /// The case the CLI itself creates. `run X --rank 0` prints
    /// "started atlas-X-rank0" and then "logs: atlasctl logs X --follow"
    /// (run.rs:195) -- a command that guessed `atlas-X` and denied the
    /// container existed, one line after saying it had started.
    #[test]
    fn a_rank_container_is_found_by_label_after_the_name_guess_misses() {
        let r = RecordingRunner::new();
        // exact-name probe: nothing
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        // label lookup: the rank container run actually created
        r.push_result(Output {
            status: 0,
            stdout: "atlas-x-rank0\n".into(),
            stderr: String::new(),
        });
        // `docker logs` itself
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });

        logs_with(&r, &args("x"), "x").expect("must stream the rank container");
        let last = r.calls().last().cloned().unwrap_or_default();
        assert!(
            last.contains(&"atlas-x-rank0".to_string()),
            "must read the container the label found, got: {last:?}"
        );
    }

    /// The label carries the RESOLVED recipe, so the lookup must filter on that
    /// and not on what the operator typed.
    ///
    /// `logs @registry/q` would otherwise query `label=…=@registry/q` against a
    /// label of `q` and still deny the container — one of the two cases the
    /// label fallback exists to fix, and the one the first version missed
    /// because it passed `args.recipe` through.
    #[test]
    fn the_label_query_uses_the_resolved_name_not_the_typed_one() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        r.push_result(Output {
            status: 0,
            stdout: "atlas-q-rank0\n".into(),
            stderr: String::new(),
        });
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });

        logs_with(&r, &args("@registry/q"), "q").expect("must find it by the resolved label");
        let label_call = &r.calls()[1];
        assert!(
            label_call.iter().any(|a| a.ends_with("=q")),
            "must filter on the resolved name: {label_call:?}"
        );
        assert!(
            !label_call.iter().any(|a| a.contains("@registry")),
            "must not filter on the typed name: {label_call:?}"
        );
    }

    /// Live and dead together must not be reported as "(none running)".
    ///
    /// This is the case the fix's own doc-comment named and no test covered,
    /// which is why the inverted flag shipped green: `any_running` was set false
    /// whenever widening ADDED anything, so two live ranks plus one crashed one
    /// claimed nothing was running. The multi-match arm needs two containers to
    /// be reached at all, so every mixed case said it.
    #[test]
    fn live_and_dead_together_is_not_reported_as_none_running() {
        let r = RecordingRunner::new();
        let out = |s: &str| Output {
            status: 0,
            stdout: s.to_owned(),
            stderr: String::new(),
        };
        r.push_result(out("")); // exact name: nothing
        r.push_result(out("atlas-x-rank0\natlas-x-rank1\n")); // running: two
        r.push_result(out("atlas-x-rank0\natlas-x-rank1\natlas-x-rank2\n")); // widened: three

        let err = logs_with(&r, &args("x"), "x").expect_err("several containers: must list them");
        let msg = format!("{err:#}");
        assert!(msg.contains("has 3 containers"), "got: {msg}");
        assert!(
            !msg.contains("none running"),
            "two of them ARE running: {msg}"
        );
    }

    /// A CRASHED container is found, and only by widening.
    ///
    /// The lookup asks for RUNNING containers first and widens to `-a` only if that
    /// does not land on exactly one. Asking with `-a` up front counted a dead rank
    /// alongside a live one, which both made the multi-match message claim "is
    /// running" about a corpse and bailed where the old code streamed the live one.
    #[test]
    fn a_crashed_container_is_found_by_widening_to_exited() {
        let r = RecordingRunner::new();
        let empty = || Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        };
        r.push_result(empty()); // exact name: nothing
        r.push_result(empty()); // label, running only: nothing
        r.push_result(Output {
            status: 0,
            stdout: "atlas-x-rank0\n".into(),
            stderr: String::new(),
        }); // label, widened to -a
        r.push_result(empty()); // docker logs

        logs_with(&r, &args("x"), "x").expect("must stream the crashed container");

        let running_first = &r.calls()[1];
        assert!(
            !running_first.contains(&"-a".to_string()),
            "the first label query must be running-only: {running_first:?}"
        );
        let widened = &r.calls()[2];
        assert!(
            widened.contains(&"-a".to_string()),
            "must widen to exited when nothing is running: {widened:?}"
        );
        let streamed = r.calls().last().cloned().unwrap_or_default();
        assert!(
            streamed.contains(&"atlas-x-rank0".to_string()),
            "must read the container the widened query found: {streamed:?}"
        );
    }

    /// Several ranks on one box: naming one would be a guess about which the
    /// operator meant, and the ranks do not log the same thing.
    #[test]
    fn several_ranks_are_listed_rather_than_chosen_between() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        r.push_result(Output {
            status: 0,
            stdout: "atlas-x-rank0\natlas-x-rank1\n".into(),
            stderr: String::new(),
        }); // running
        // Scripted deliberately. Without it the widened query consumed
        // RecordingRunner's default-empty result, so the test passed on a
        // response nobody wrote.
        r.push_result(Output {
            status: 0,
            stdout: "atlas-x-rank0\natlas-x-rank1\n".into(),
            stderr: String::new(),
        }); // widened: the same two
        let err = logs_with(&r, &args("x"), "x").expect_err("must refuse to pick one");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("atlas-x-rank0") && msg.contains("atlas-x-rank1"),
            "got: {msg}"
        );
        assert!(
            msg.contains("docker logs"),
            "must say how to read one: {msg}"
        );
    }

    /// Nothing under that name offers BOTH explanations, and asserts neither.
    ///
    /// The message used to state "it has not been started here" as fact. That is
    /// false in the case an operator is most likely to be in: recipes run with
    /// `--rm`, so a container that started and then died is removed, and looking
    /// for its logs is what brought them here. Being told the launch never
    /// happened contradicts the "started" they had just read.
    #[test]
    fn nothing_under_that_name_offers_both_explanations() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        let err = logs_with(&r, &args("x"), "x").expect_err("must refuse");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("never started here"),
            "must offer the never-started case: {msg}"
        );
        assert!(
            msg.contains("--rm"),
            "must offer the started-and-removed case, which is the likelier one \
             for somebody reading logs: {msg}"
        );
        assert!(
            !msg.contains("it has not been started here"),
            "must not assert a cause it cannot know: {msg}"
        );
        // rustfmt reflows `\`-continued strings into runs of spaces inside the
        // message body; it has done so three times in this session, and it is
        // invisible in review.
        assert!(
            !msg.replace("\n    ", "").contains("   "),
            "reflowed whitespace leaked into the message: {msg:?}"
        );
    }
}
