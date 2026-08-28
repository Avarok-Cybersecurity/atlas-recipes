// SPDX-License-Identifier: AGPL-3.0-only

//! The Windows half of the plan: a Task Scheduler task at logon.
//!
//! Split from [`super`] for size, along the seam the platform already draws.
//! Everything here is pure for the same reason the rest of the planner is —
//! the interesting failures are quoting and scheduler settings, and both are
//! testable without a Windows machine.

use super::{AgentInvocation, SERVICE_NAME, ServiceKind, ServicePlan, xml_escape};
use std::path::{Path, PathBuf};

/// Where the Windows task's output is kept.
///
/// Task Scheduler captures nothing, so without this an agent that exits at
/// startup leaves no trace anywhere an operator would look — the failure that
/// `agent install` prints an explicit diagnostic for on the other two
/// platforms would be undiagnosable here.
const WINDOWS_LOG_FILE: &str = "atlasctl-agent.log";

/// The Windows log file's full path, derived once so the plan and the advice
/// that points at it cannot disagree.
#[must_use]
pub fn windows_log_path(home: &Path) -> PathBuf {
    home.join("AppData")
        .join("Local")
        .join("atlasctl")
        .join(WINDOWS_LOG_FILE)
}

pub(super) fn scheduled_task(agent: &AgentInvocation, home: &Path) -> ServicePlan {
    let dir = home.join("AppData").join("Local").join("atlasctl");
    let unit_path = dir.join(format!("{SERVICE_NAME}.xml"));
    let log = windows_log_path(home);

    // Run THROUGH cmd so the agent's output lands somewhere. Task Scheduler's
    // Exec action has no redirection of its own and captures nothing, so the
    // alternative is an agent that exits at startup and leaves no trace.
    //
    // The doubled outer quotes are cmd's documented form: given `/c "..."` it
    // strips one outer pair, so the inner quoting is what the agent sees.
    let inner = format!(
        "{} >> {} 2>&1",
        windows_command_line(&agent.argv()),
        windows_quote(&log.display().to_string())
    );
    let args = format!("/c \"{inner}\"");

    // ExecutionTimeLimit PT0S is the whole reason this is XML rather than
    // `schtasks /Create /SC ONLOGON`: that route bakes in a 72-hour limit, and
    // a forever-process that Task Scheduler kills every three days looks like a
    // crash nobody can reproduce.
    let unit_body = format!(
        "<?xml version=\"1.0\"?>\n\
         <Task version=\"1.4\" \
         xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
         \x20 <RegistrationInfo>\n\
         \x20   <Description>Atlas agent — the local control plane the website \
         talks to. Written by `atlasctl agent install`; reinstalling replaces \
         it.</Description>\n\
         \x20   <URI>\\{SERVICE_NAME}</URI>\n\
         \x20 </RegistrationInfo>\n\
         \x20 <Triggers>\n\
         \x20   <LogonTrigger><Enabled>true</Enabled></LogonTrigger>\n\
         \x20 </Triggers>\n\
         \x20 <Principals>\n\
         \x20   <Principal id=\"Author\">\n\
         \x20     <LogonType>InteractiveToken</LogonType>\n\
         \x20     <RunLevel>LeastPrivilege</RunLevel>\n\
         \x20   </Principal>\n\
         \x20 </Principals>\n\
         \x20 <Settings>\n\
         \x20   <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
         \x20   <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
         \x20   <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
         \x20   <AllowHardTerminate>true</AllowHardTerminate>\n\
         \x20   <StartWhenAvailable>true</StartWhenAvailable>\n\
         \x20   <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>\n\
         \x20   <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd>\
         <RestartOnIdle>false</RestartOnIdle></IdleSettings>\n\
         \x20   <AllowStartOnDemand>true</AllowStartOnDemand>\n\
         \x20   <Enabled>true</Enabled>\n\
         \x20   <Hidden>true</Hidden>\n\
         \x20   <RunOnlyIfIdle>false</RunOnlyIfIdle>\n\
         \x20   <WakeToRun>false</WakeToRun>\n\
         \x20   <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
         \x20   <Priority>7</Priority>\n\
         \x20   <RestartOnFailure><Interval>PT1M</Interval>\
         <Count>3</Count></RestartOnFailure>\n\
         \x20 </Settings>\n\
         \x20 <Actions Context=\"Author\">\n\
         \x20   <Exec>\n\
         \x20     <Command>cmd.exe</Command>\n\
         \x20     <Arguments>{}</Arguments>\n\
         \x20   </Exec>\n\
         \x20 </Actions>\n\
         </Task>\n",
        xml_escape(&args)
    );

    // Register through PowerShell rather than `schtasks /XML`: schtasks reads
    // the file itself and rejects anything it does not consider Unicode, while
    // `Get-Content -Raw` hands the API a string and the file's encoding stops
    // mattering.
    let ps = |script: &str| {
        vec![
            "powershell".to_owned(),
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            script.to_owned(),
        ]
    };
    let quoted_unit = ps_single_quote(&unit_path.display().to_string());

    ServicePlan {
        kind: ServiceKind::ScheduledTask,
        unit_path,
        unit_body,
        // Ending a running instance before replacing the definition, for the
        // same reason launchd needs a bootout: -Force replaces the task but
        // leaves the OLD process running, so an upgrade would report success
        // while the previous binary kept the port.
        pre_activate: vec![ps(&format!(
            "Stop-ScheduledTask -TaskName '{SERVICE_NAME}' -ErrorAction Stop"
        ))],
        activate: vec![
            ps(&format!(
                "Register-ScheduledTask -TaskName '{SERVICE_NAME}'                  -Xml (Get-Content -Raw -Path {quoted_unit}) -Force | Out-Null"
            )),
            ps(&format!("Start-ScheduledTask -TaskName '{SERVICE_NAME}'")),
        ],
        // The same question `is-active` answers: the task EXISTING is not the
        // agent running, and `Get-ScheduledTask` alone would report success for
        // a task whose process died on startup.
        // Polled, not sampled once. `Start-ScheduledTask` returns as soon as
        // the scheduler has ACCEPTED the request, not when the process is up,
        // so a single read right after it reports `Ready` and the install
        // announces "installed, but it is NOT running" for an agent that is
        // about to be. That is what a REINSTALL did on CI, because a replaced
        // task has to be stopped and started rather than merely started. Ten
        // seconds is long enough for that, and short enough that a genuinely
        // crash-looping agent is still reported as one.
        verify: ps(&format!(
            "$deadline = (Get-Date).AddSeconds(10); \
             do {{ \
               if ((Get-ScheduledTask -TaskName '{SERVICE_NAME}' \
                    -ErrorAction SilentlyContinue).State -eq 'Running') {{ exit 0 }}; \
               Start-Sleep -Milliseconds 250 \
             }} while ((Get-Date) -lt $deadline); \
             exit 1"
        )),
        best_effort: Vec::new(),
        deactivate: vec![ps(&format!(
            "Unregister-ScheduledTask -TaskName '{SERVICE_NAME}' -Confirm:$false"
        ))],
    }
}

/// Quote one argument the way `CommandLineToArgvW` parses it.
///
/// Not the same rules as a shell: a backslash is literal EXCEPT immediately
/// before a quote, where it escapes. Getting this wrong turns
/// `C:\Users\me\bin\` into an unterminated quote and the whole command line
/// into one unusable argument.
fn windows_quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_owned();
    }
    let mut out = String::with_capacity(a.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Double the run, then escape the quote itself.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                backslashes = 0;
                out.push('"');
                continue;
            }
            _ => backslashes = 0,
        }
        out.push(c);
    }
    // A trailing run would otherwise escape the closing quote.
    out.extend(std::iter::repeat_n('\\', backslashes));
    out.push('"');
    out
}

/// Join an argv into one Windows command line.
fn windows_command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|a| windows_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote a value inside a PowerShell single-quoted string.
fn ps_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
