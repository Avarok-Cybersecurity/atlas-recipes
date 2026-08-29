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
        r.push_result(out(0, "\n", "")); // ...and neither did the label lookup
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
        // Asserted directly rather than by counting calls. The count was 1 when
        // this only guessed a name; it now also asks the LABEL before concluding
        // nothing is running, because a rank launch is `atlas-q-rank0` and the
        // guess never finds it. "Did not stream" is the actual claim, and saying
        // it outright survives that lookup being added.
        assert!(
            !r.calls().iter().any(|c| c.contains(&"logs".to_string())),
            "must not have run `docker logs` at all: {:?}",
            r.calls()
        );
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

mod daemon_down {
    use super::super::*;
    use atlasctl_core::io::RecordingRunner;
    use atlasctl_core::io::process::Output;

    /// An empty list because docker did not ANSWER is not an idle fleet.
    /// `stop --all` against a stopped daemon printed "nothing running" and
    /// exited 0 — the exact lie the collected-failure logic exists to prevent.
    #[test]
    fn stop_all_against_a_dead_daemon_is_not_an_idle_fleet() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 1,
            stdout: String::new(),
            stderr: "Cannot connect to the Docker daemon at unix:///var/run/docker.sock".into(),
        });
        let e = stop_all_with(&r).expect_err("a daemon that never answered is not an empty fleet");
        assert!(e.to_string().contains("docker ps"), "{e}");
    }

    /// A real empty answer still reads as empty.
    #[test]
    fn a_daemon_that_answers_with_nothing_is_an_idle_fleet() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "\n".into(),
            stderr: String::new(),
        });
        stop_all_with(&r).expect("an answered-but-empty listing is genuinely idle");
    }
}

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

        logs_with(&r, &args("x")).expect("must stream the rank container");
        let last = r.calls().last().cloned().unwrap_or_default();
        assert!(
            last.contains(&"atlas-x-rank0".to_string()),
            "must read the container the label found, got: {last:?}"
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
        });
        let err = logs_with(&r, &args("x")).expect_err("must refuse to pick one");
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

    /// Nothing running under that name at all keeps the original message.
    #[test]
    fn genuinely_absent_still_says_it_was_never_started_here() {
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
        let err = logs_with(&r, &args("x")).expect_err("must refuse");
        assert!(format!("{err:#}").contains("has not been started here"));
    }
}
