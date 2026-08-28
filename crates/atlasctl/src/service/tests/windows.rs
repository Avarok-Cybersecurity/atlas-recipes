// SPDX-License-Identifier: AGPL-3.0-only

//! The Windows plan's tests, split from [`super`] for size.

use crate::service::plan::{ServiceKind, plan};
use std::path::{Path, PathBuf};

use super::agent;

// ---- the Windows plan --------------------------------------------------------

#[test]
fn a_windows_task_has_no_execution_time_limit() {
    let p = plan(
        ServiceKind::ScheduledTask,
        &agent(),
        Path::new("C:\\Users\\o"),
        0,
    );
    // `schtasks /Create /SC ONLOGON` bakes in 72 hours. A forever-process that
    // the scheduler kills every three days is a crash nobody can reproduce,
    // and this element is the entire reason the plan writes XML instead.
    assert!(
        p.unit_body
            .contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"),
        "{}",
        p.unit_body
    );
}

/// A laptop is the machine this is most likely to be installed on, so a task
/// that refuses to start on battery would fail exactly where it is needed.
#[test]
fn a_windows_task_runs_on_battery() {
    let p = plan(
        ServiceKind::ScheduledTask,
        &agent(),
        Path::new("C:\\Users\\o"),
        0,
    );
    assert!(p.unit_body.contains("<DisallowStartIfOnBatteries>false<"));
    assert!(p.unit_body.contains("<StopIfGoingOnBatteries>false<"));
}

/// Task Scheduler's Exec action captures no output at all. Without a log an
/// agent that exits at startup leaves nothing anywhere an operator would look,
/// and `agent install` prints advice naming a file that would never exist.
#[test]
fn a_windows_task_keeps_a_log_where_the_advice_says_it_is() {
    let home = Path::new("C:\\Users\\o");
    let p = plan(ServiceKind::ScheduledTask, &agent(), home, 0);
    let log = crate::service::plan::windows_log_path(home);
    let leaf = log.file_name().expect("a file name").to_string_lossy();
    assert!(p.unit_body.contains("--log-file"), "{}", p.unit_body);
    assert!(p.unit_body.contains(&*leaf), "{}", p.unit_body);
}

/// The agent must be the task's OWN process. A shell wrapper that redirects
/// for it looks equivalent and is not: Task Scheduler's stop terminates only
/// the process it started, so the wrapper dies, the agent is orphaned still
/// holding the port, and the replacement exits at startup — which is exactly
/// what a reinstall did on CI, reporting "installed, but it is NOT running".
#[test]
fn a_windows_task_runs_the_agent_directly_and_not_through_a_shell() {
    let p = plan(
        ServiceKind::ScheduledTask,
        &agent(),
        Path::new("C:\\Users\\o"),
        0,
    );
    for shell in ["cmd.exe", "cmd ", "/c "] {
        assert!(
            !p.unit_body.contains(shell),
            "the task must not run through {shell}: {}",
            p.unit_body
        );
    }
    assert!(p.unit_body.contains("atlasctl"), "{}", p.unit_body);
}

/// Replacing the task definition leaves the OLD process running, so an upgrade
/// would report success while the previous binary still held the port — the
/// same failure `bootout` fixes on macOS.
#[test]
fn a_windows_upgrade_ends_the_running_instance_first() {
    let p = plan(
        ServiceKind::ScheduledTask,
        &agent(),
        Path::new("C:\\Users\\o"),
        0,
    );
    let pre = p
        .pre_activate
        .iter()
        .map(|a| a.join(" "))
        .collect::<Vec<_>>();
    assert!(
        pre.iter().any(|c| c.contains("Stop-ScheduledTask")),
        "{pre:?}"
    );
    let act = p.activate.iter().map(|a| a.join(" ")).collect::<Vec<_>>();
    assert!(act.iter().any(|c| c.contains("-Force")), "{act:?}");
}

/// The task existing is not the agent running. `Get-ScheduledTask` alone exits
/// 0 for a task whose process died at startup, which is precisely the state
/// the verify step exists to catch.
#[test]
fn a_windows_verify_asks_whether_it_is_running_not_whether_it_exists() {
    let p = plan(
        ServiceKind::ScheduledTask,
        &agent(),
        Path::new("C:\\Users\\o"),
        0,
    );
    let v = p.verify.join(" ");
    assert!(v.contains("-eq 'Running'"), "{v}");
    assert!(v.contains("exit 1"), "must fail when it is not: {v}");
    // And it must WAIT. `Start-ScheduledTask` returns when the scheduler has
    // accepted the request, not when the process is up, so a single read told
    // an operator their freshly upgraded agent was down — observed on CI, on
    // the reinstall, which is the path this whole branch exists to fix.
    assert!(
        v.contains("Start-Sleep") && v.contains("while"),
        "a one-shot read races the scheduler: {v}"
    );
}

/// `CommandLineToArgvW` treats a backslash as literal EXCEPT before a quote.
/// A Windows install path ends in a backslash more often than not, and getting
/// this wrong turns the whole command line into one unusable argument.
#[test]
fn a_path_ending_in_a_backslash_survives_quoting() {
    let a = crate::service::plan::AgentInvocation {
        exe: PathBuf::from("C:\\Program Files\\Atlas\\atlasctl.exe"),
        port: 34333,
        client: false,
        discovery: true,
        browser: true,
        config_dir: Some(PathBuf::from("C:\\Users\\o\\state dir\\")),
        log_file: None,
    };
    let p = plan(ServiceKind::ScheduledTask, &a, Path::new("C:\\Users\\o"), 0);
    // The trailing backslash must be doubled so it escapes itself rather than
    // the closing quote.
    assert!(
        p.unit_body.contains("state dir\\\\&quot;") || p.unit_body.contains("state dir\\\\\""),
        "{}",
        p.unit_body
    );
}

// ---- the I/O half, on Windows -----------------------------------------------
//
// `install` resolves the supervisor itself, so these can only run where that
// resolution yields a Scheduled Task. Their absence is what made `Files::body`
// read as dead code on Windows — the honest fix for which is the coverage, not
// an allow.

#[cfg(target_os = "windows")]
use super::{Files, Recorder, home};
#[cfg(target_os = "windows")]
use crate::service::{install, uninstall};

#[cfg(target_os = "windows")]
#[test]
fn a_successful_install_writes_the_task_then_registers_it() {
    let fs = Files::default();
    let r = Recorder::new();
    let out = install(&fs, &r, &agent(), &home(), 0).expect("installs");

    assert_eq!(
        out.unit_path,
        home().join("AppData\\Local\\atlasctl\\atlasctl-agent.xml")
    );
    assert!(fs.body().contains("<ExecutionTimeLimit>"), "{}", fs.body());
    assert!(out.skipped.is_empty());
    let calls = r.calls();
    // Register before Start, and the stop attempt before either: starting a
    // task that has not been registered fails, and registering over a live one
    // leaves the old process holding the port.
    let stop = calls.iter().position(|c| c.contains("Stop-ScheduledTask"));
    let reg = calls
        .iter()
        .position(|c| c.contains("Register-ScheduledTask"))
        .expect("must register");
    let start = calls
        .iter()
        .position(|c| c.contains("Start-ScheduledTask"))
        .expect("must start");
    assert!(reg < start, "{calls:?}");
    assert!(stop.is_none_or(|s| s < reg), "{calls:?}");
}

/// The first install on a machine has no task to stop, so `Stop-ScheduledTask`
/// fails — and must not fail the install. This is the Windows form of the bug
/// that made `launchctl bootstrap` refuse every second macOS install.
#[cfg(target_os = "windows")]
#[test]
fn a_first_install_survives_having_nothing_to_stop() {
    let fs = Files::default();
    let r = Recorder::failing("Stop-ScheduledTask");
    install(&fs, &r, &agent(), &home(), 0).expect("a first install must not need a running task");
}

/// Registration failing must fail the install and name the step, rather than
/// reporting an installed agent that Task Scheduler never accepted.
#[cfg(target_os = "windows")]
#[test]
fn a_failed_registration_fails_the_install_and_names_itself() {
    let fs = Files::default();
    let r = Recorder::failing("Register-ScheduledTask");
    let e = install(&fs, &r, &agent(), &home(), 0).expect_err("must fail");
    let msg = format!("{e:#}");
    assert!(msg.contains("Register-ScheduledTask"), "{msg}");
    // The XML stays on disk: it is the evidence for why registration failed.
    assert_eq!(fs.written.lock().expect("lock").len(), 1);
}

#[cfg(target_os = "windows")]
#[test]
fn uninstall_unregisters_the_task_before_deleting_its_definition() {
    let fs = Files::default();
    let r = Recorder::new();
    let path = uninstall(&fs, &r, &home(), 0).expect("uninstalls");
    assert_eq!(
        path,
        home().join("AppData\\Local\\atlasctl\\atlasctl-agent.xml")
    );
    assert!(
        r.calls()[0].contains("Unregister-ScheduledTask"),
        "{:?}",
        r.calls()
    );
    assert_eq!(*fs.removed.lock().expect("lock"), vec![path]);
}

/// Uninstalling twice, or after a half-finished install, is the ordinary case.
#[cfg(target_os = "windows")]
#[test]
fn uninstalling_something_that_was_never_installed_succeeds() {
    let fs = Files::default();
    let r = Recorder::failing("Unregister-ScheduledTask");
    uninstall(&fs, &r, &home(), 0).expect("must not fail");
}
