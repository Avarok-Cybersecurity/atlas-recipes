// SPDX-License-Identifier: AGPL-3.0-only

//! Why an agent can pick up a launch it did not start.
//!
//! Nothing here is remembered from a launch this process performed. The
//! container and the port are asked of the container runtime every time, which
//! is what lets a restarted agent adopt what its predecessor left running —
//! and what stops it reporting another model's numbers under this one's name.

use super::*;
use atlasctl_agent::launchstats::MetricsSource;
use atlasctl_core::io::RecordingRunner;
use atlasctl_core::io::process::Output;
use std::sync::Mutex as StdMutex;

const RECIPE: &str = "qwen3.6-35b-a3b-fp8-bf16head";
const NAME: &str = "atlas-qwen3.6-35b-a3b-fp8-bf16head";

fn ok(stdout: &str) -> Output {
    Output {
        status: 0,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

/// Records which port it was asked to scrape.
struct PortSpy(StdMutex<Vec<u16>>);

impl MetricsSource for PortSpy {
    fn scrape(&self, port: u16) -> anyhow::Result<String> {
        self.0.lock().expect("lock").push(port);
        Ok("atlas_requests_total 7\n".to_owned())
    }
}

fn recipe_id() -> RecipeId {
    RecipeId::parse(RECIPE).expect("a valid id")
}

fn telemetry(runner: &Arc<RecordingRunner>) -> LocalLaunchTelemetry {
    LocalLaunchTelemetry::new(
        Arc::clone(runner) as Arc<dyn ProcessRunner>,
        atlasctl_agent::launchstats::LaunchSampler::new(Box::new(PortSpy(StdMutex::new(
            Vec::new(),
        )))),
    )
}

/// The property re-adoption rests on: the port comes from the container that is
/// actually running, not from the recipe. A launch can override the port, and
/// trusting the recipe would scrape whatever else was on the default and report
/// another model's throughput under this one's name.
#[test]
fn the_port_is_read_from_the_running_container_not_the_recipe() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(&format!("{NAME}\trunning\n")));
    runner.push_result(ok("serve\n--port\n9911\n--host\n0.0.0.0\n"));

    let t = telemetry(&runner);
    let reading = t.sample(&recipe_id()).expect("samples");
    assert_eq!(reading.requests_total, Some(7.0));

    // The port reached the scraper by way of the container's own arguments,
    // which is the only place it could have come from: the recipe was never
    // consulted.
    let calls = runner.calls();
    assert_eq!(calls[0][1], "ps", "it must ask what is running first");
    assert!(calls[1].contains(&"inspect".to_owned()), "{calls:?}");
    assert!(calls[1].contains(&NAME.to_owned()), "{calls:?}");
}

#[test]
fn an_equals_form_port_is_understood_too() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(&format!("{NAME}\trunning\n")));
    runner.push_result(ok("serve\n--port=9911\n"));
    let t = telemetry(&runner);
    assert!(t.sample(&recipe_id()).is_ok());
}

/// Guessing would scrape somebody else's server and label the numbers with this
/// recipe's name.
#[test]
fn a_launch_with_no_port_is_refused_rather_than_guessed() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(&format!("{NAME}\trunning\n")));
    runner.push_result(ok("serve\n--host\n0.0.0.0\n"));
    let t = telemetry(&runner);
    let err = t.sample(&recipe_id()).expect_err("no port to scrape");
    assert!(err.contains("does not name a port"), "{err}");
}

#[test]
fn a_recipe_that_was_never_launched_here_says_so() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(""));
    let t = telemetry(&runner);
    let err = t.sample(&recipe_id()).expect_err("nothing launched");
    assert!(err.contains("has not been launched"), "{err}");
}

/// A launch that has stopped is exactly the one whose log an operator wants.
/// Listing only running containers would answer "no such launch" at the moment
/// the question matters most.
#[test]
fn logs_are_available_from_a_container_that_has_exited() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(&format!("{NAME}\texited\n")));
    runner.push_result(Output {
        status: 0,
        stdout: String::new(),
        stderr: "Error: Snapshot has no weight files\n".to_owned(),
    });
    let t = telemetry(&runner);

    let tail = t.logs(&recipe_id(), 50).expect("reads the log");
    assert_eq!(tail.container, NAME);
    assert!(!tail.running, "the container has exited");
    // The interesting half of a failed start is usually the stderr half.
    assert_eq!(tail.lines, vec!["Error: Snapshot has no weight files"]);

    assert!(
        runner.calls()[0].contains(&"-a".to_owned()),
        "stopped containers must be listed: {:?}",
        runner.calls()[0]
    );
}

/// Sampling is refused for a launch that has stopped: a rate needs a live
/// engine, and the last scrape of a dead one is not news.
#[test]
fn stats_are_refused_for_a_container_that_has_exited() {
    let runner = Arc::new(RecordingRunner::new());
    runner.push_result(ok(&format!("{NAME}\texited\n")));
    let t = telemetry(&runner);
    let err = t.sample(&recipe_id()).expect_err("not running");
    assert!(err.contains("is not running"), "{err}");
}
