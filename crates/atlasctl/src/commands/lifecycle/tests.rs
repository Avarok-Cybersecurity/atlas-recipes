// SPDX-License-Identifier: AGPL-3.0-only

//! Tests for `lifecycle`, moved out when the file reached the 500-line cap.
//! Split verbatim: the module bodies are unchanged, only their `#[cfg(test)]`
//! attributes moved to the one declaration in the parent.

mod naming {
    use super::super::*;
    use atlasctl_core::docker::translate::LABEL_RECIPE;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    #[test]
    fn managed_containers_are_found_by_label_not_by_name_guessing() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        let names = managed_containers(&r).unwrap();
        assert_eq!(names, ["atlas-a", "atlas-b"]);
        let argv = &r.calls()[0];
        assert!(
            argv.contains(&format!("label={LABEL_MANAGED}=1")),
            "must filter by our label so unrelated containers are never touched: {argv:?}"
        );
    }

    #[test]
    fn the_recipe_label_is_what_ties_a_container_back_to_its_recipe() {
        assert_eq!(LABEL_RECIPE, "io.atlasctl.recipe");
        assert_eq!(container_of("qwen3.6-27b-fp8"), "atlas-qwen3.6-27b-fp8");
    }
}

mod stop_tests {
    use super::super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    fn ok() -> Output {
        Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
    fn fail(why: &str) -> Output {
        Output {
            status: 1,
            stdout: String::new(),
            stderr: why.into(),
        }
    }

    /// The bug: every stop failed and the command exited 0, so a script that
    /// checked the exit code — or an operator running this before a reboot —
    /// was told the fleet was idle while every container was still up.
    #[test]
    fn a_failed_stop_is_a_failed_command() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        r.push_result(fail("permission denied"));
        // Was "no such container", which is now the one stderr that means the
        // stop SUCCEEDED in effect. This test is about two real failures, so it
        // needs two real failures.
        r.push_result(fail("device or resource busy"));

        let e = stop_all_with(&r).expect_err("two failures must not report success");
        let msg = e.to_string();
        assert!(msg.contains("atlas-a") && msg.contains("atlas-b"), "{msg}");
    }

    /// A partial failure is still a failure, and the survivors are named —
    /// "1 of 3 failed" sends the operator to check all three.
    #[test]
    fn a_partial_failure_names_only_what_did_not_stop() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "atlas-a\natlas-b\n".into(),
            stderr: String::new(),
        });
        r.push_result(ok());
        r.push_result(fail("device busy"));

        let e = stop_all_with(&r).expect_err("one failure is a failure");
        let msg = e.to_string();
        assert!(msg.contains("atlas-b"), "{msg}");
        assert!(
            !msg.contains("atlas-a"),
            "the one that stopped is not a problem: {msg}"
        );
    }

    #[test]
    fn stopping_everything_when_nothing_runs_is_success() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        });
        stop_all_with(&r).expect("nothing to do is fine");
    }
}

mod absent_tests {
    use super::super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    fn out(status: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[test]
    fn the_daemons_two_spellings_of_gone_are_both_recognised() {
        assert!(absent(
            "Error response from daemon: No such container: atlas-x"
        ));
        assert!(absent("Error: no such object: atlas-x"));
        // Real failures must not be swallowed by this.
        assert!(!absent("permission denied"));
        assert!(!absent("device or resource busy"));
        assert!(!absent(""));
    }

    /// Stopping something that is not running is the state the operator asked
    /// for. It used to print the daemon's own line and exit non-zero.
    #[test]
    fn stopping_a_recipe_that_is_not_running_is_not_a_failure() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "\n", "")); // the label query matched nothing
        stop_recipe_with(&r, "q", "q").expect("a recipe that is not running must not be an error");
        assert_eq!(r.calls().len(), 1, "must not have run `docker stop` at all");
    }

    /// The bug the label query exists to remove. A cluster launch names its
    /// container `atlas-{recipe}-rank{n}`, and `run` tells the operator to stop
    /// it with `atlasctl stop {recipe}`. Guessing `atlas-{recipe}` missed, the
    /// daemon said "No such container", and — once that stopped being a failure
    /// — stop reported exit 0 while the rank was still serving.
    #[test]
    fn a_rank_container_is_found_by_label_not_by_the_name_we_would_have_guessed() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-q-rank0\n", ""));
        r.push_result(out(0, "", ""));
        stop_recipe_with(&r, "q", "q").expect("the rank container must be stopped");
        let query = &r.calls()[0];
        assert!(
            query.contains(&format!("label={LABEL_RECIPE}=q")),
            "found by label: {query:?}"
        );
        assert!(
            r.calls()[1].contains(&"atlas-q-rank0".to_string()),
            "stops the container that actually exists, not `atlas-q`: {:?}",
            r.calls()[1]
        );
    }

    /// A scoped ref resolves to the recipe's own name, so the label query is
    /// asked the right question. Guessing produced `atlas-@reg/q`.
    #[test]
    fn a_scoped_ref_asks_for_the_recipes_own_name() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-q\n", ""));
        r.push_result(out(0, "", ""));
        stop_recipe_with(&r, "@reg/q", "q").expect("stops");
        assert!(
            r.calls()[0].contains(&format!("label={LABEL_RECIPE}=q")),
            "{:?}",
            r.calls()[0]
        );
    }

    /// Under --all this is a race: the container ended between the listing and
    /// the stop. Equally not a failure, and it must not mask a real one.
    #[test]
    fn a_vanished_container_does_not_mask_a_real_failure() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-a\natlas-b\n", ""));
        r.push_result(out(
            1,
            "",
            "Error response from daemon: No such container: atlas-a",
        ));
        r.push_result(out(1, "", "permission denied"));
        let e = stop_all_with(&r).expect_err("the real failure must still fail the command");
        let msg = e.to_string();
        assert!(msg.contains("atlas-b"), "names what actually failed: {msg}");
        assert!(
            !msg.contains("atlas-a"),
            "must not blame the one that was already gone: {msg}"
        );
    }

    #[test]
    fn logs_for_a_recipe_never_started_here_says_so_and_never_streams() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "\n", "")); // ps -a matched nothing
        let e = logs_with(
            &r,
            &LogsArgs {
                recipe: "q".into(),
                tail: 100,
                follow: false,
            },
        )
        .expect_err("there is nothing to show");
        let msg = e.to_string();
        assert!(msg.contains("has not been started here"), "{msg}");
        assert!(
            msg.contains("atlasctl status"),
            "points somewhere useful: {msg}"
        );
        assert_eq!(r.calls().len(), 1, "must not have run `docker logs` at all");
    }

    #[test]
    fn logs_for_an_exited_container_still_stream() {
        let r = RecordingRunner::new();
        r.push_result(out(0, "atlas-q\n", "")); // ps -a found it, exited or not
        r.push_result(out(0, "", ""));
        logs_with(
            &r,
            &LogsArgs {
                recipe: "q".into(),
                tail: 100,
                follow: false,
            },
        )
        .expect("a stopped container still has logs worth reading");
        let argv = &r.calls()[1];
        assert!(argv.contains(&"logs".to_string()), "{argv:?}");
    }
}
