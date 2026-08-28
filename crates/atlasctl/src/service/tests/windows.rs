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

/// Task Scheduler's Exec action captures no output at all. Without the
/// redirect an agent that exits at startup leaves nothing anywhere an operator
/// would look, and `agent install` prints advice pointing at a file that would
/// never exist.
#[test]
fn a_windows_task_keeps_a_log_where_the_advice_says_it_is() {
    let home = Path::new("C:\\Users\\o");
    let p = plan(ServiceKind::ScheduledTask, &agent(), home, 0);
    let log = crate::service::plan::windows_log_path(home);
    let leaf = log.file_name().expect("a file name").to_string_lossy();
    assert!(p.unit_body.contains(&*leaf), "{}", p.unit_body);
    assert!(p.unit_body.contains("2&gt;&amp;1"), "{}", p.unit_body);
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
