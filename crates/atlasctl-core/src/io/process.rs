// SPDX-License-Identifier: AGPL-3.0-only

//! Running external programs.

use anyhow::{Context, Result};

/// What a finished process produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Exit status, or -1 if the process was killed by a signal.
    pub status: i32,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

impl Output {
    /// Whether the process exited zero.
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

/// Runs external programs.
///
/// Every method takes an argv vector, never a command string. There is no shell
/// anywhere in this trait or its implementations, so a hostile value in a
/// recipe or a request can only ever be one inert argument.
pub trait ProcessRunner: Send + Sync {
    /// Run to completion, capturing output.
    fn run(&self, argv: &[String]) -> Result<Output>;

    /// Run with stdio inherited, for streaming output like `docker logs -f`.
    fn run_streaming(&self, argv: &[String]) -> Result<i32>;
}

/// The real implementation, over `std::process`.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdProcessRunner;

impl ProcessRunner for StdProcessRunner {
    fn run(&self, argv: &[String]) -> Result<Output> {
        let (program, args) = argv.split_first().context("empty argv")?;
        let out = std::process::Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("failed to run `{program}`"))?;
        Ok(Output {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn run_streaming(&self, argv: &[String]) -> Result<i32> {
        let (program, args) = argv.split_first().context("empty argv")?;
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("failed to run `{program}`"))?;
        Ok(status.code().unwrap_or(-1))
    }
}

#[cfg(any(test, feature = "test-mocks"))]
mod mock {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Records every invocation and replays scripted results.
    ///
    /// Hand-written rather than generated: the assertions we care about are
    /// "exactly this argv, in exactly this order", and a recording struct says
    /// that more plainly than a matcher DSL.
    #[derive(Debug, Default)]
    pub struct RecordingRunner {
        calls: Mutex<Vec<Vec<String>>>,
        scripted: Mutex<VecDeque<Output>>,
    }

    impl RecordingRunner {
        /// A runner that returns success with empty output for every call.
        pub fn new() -> Self {
            Self::default()
        }

        /// Queue a result for the next call.
        pub fn push_result(&self, output: Output) {
            self.scripted.lock().expect("lock").push_back(output);
        }

        /// Every argv this runner was asked to run, in order.
        pub fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().expect("lock").clone()
        }

        /// How many times it was called.
        pub fn call_count(&self) -> usize {
            self.calls.lock().expect("lock").len()
        }

        fn record(&self, argv: &[String]) -> Output {
            self.calls.lock().expect("lock").push(argv.to_vec());
            self.scripted
                .lock()
                .expect("lock")
                .pop_front()
                .unwrap_or(Output {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
        }
    }

    impl ProcessRunner for RecordingRunner {
        fn run(&self, argv: &[String]) -> Result<Output> {
            Ok(self.record(argv))
        }

        fn run_streaming(&self, argv: &[String]) -> Result<i32> {
            Ok(self.record(argv).status)
        }
    }
}

#[cfg(any(test, feature = "test-mocks"))]
pub use mock::RecordingRunner;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mock_records_argv_verbatim() {
        let r = RecordingRunner::new();
        r.run(&["docker".into(), "ps".into(), "-a".into()]).unwrap();
        assert_eq!(r.calls(), [["docker", "ps", "-a"]]);
    }

    #[test]
    fn scripted_results_are_returned_in_order() {
        let r = RecordingRunner::new();
        r.push_result(Output {
            status: 0,
            stdout: "first".into(),
            stderr: String::new(),
        });
        r.push_result(Output {
            status: 1,
            stdout: "second".into(),
            stderr: String::new(),
        });
        assert_eq!(r.run(&["x".into()]).unwrap().stdout, "first");
        let second = r.run(&["x".into()]).unwrap();
        assert_eq!(second.stdout, "second");
        assert!(!second.success());
    }

    #[test]
    fn an_empty_argv_is_an_error_not_a_panic() {
        assert!(StdProcessRunner.run(&[]).is_err());
    }

    #[test]
    fn the_real_runner_captures_output_without_a_shell() {
        // `echo` here is the binary, not a shell builtin, and the metacharacter
        // must come back as literal text rather than being interpreted.
        let out = StdProcessRunner
            .run(&["/bin/echo".into(), "a;b".into()])
            .expect("echo runs");
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "a;b");
    }
}
