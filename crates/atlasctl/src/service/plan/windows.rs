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

    // The agent is the task's OWN process, not a child of a shell that
    // redirects for it. That was the first shape, and it broke the upgrade
    // path: Task Scheduler's stop terminates only the process it started, so
    // the `cmd.exe` wrapper died and left the agent orphaned, still holding
    // the port, and the replacement exited at startup. `--log-file` moves the
    // redirect inside the agent, which removes the wrapper, the orphan, and a
    // layer of quoting at once.
    let mut invocation = agent.clone();
    invocation.log_file = Some(log.clone());
    let argv = invocation.argv();
    let (command, rest) = argv.split_first().expect("argv is never empty");
    let args = windows_command_line(rest);

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
         \x20     <Command>{}</Command>\n\
         \x20     <Arguments>{}</Arguments>\n\
         \x20   </Exec>\n\
         \x20 </Actions>\n\
         </Task>\n",
        xml_escape(command),
        xml_escape(&args)
    );

    // Register through PowerShell rather than `schtasks /XML`: schtasks reads
    // the file itself and rejects anything it does not consider Unicode, while
    // `Get-Content` hands the API a string.
    //
    // `-Encoding UTF8` is not optional. `powershell` is always 5.1, whose
    // `Get-Content` decodes as the system ANSI codepage when the file has no
    // BOM — and the unit is written UTF-8 without one. The body always contains
    // a multibyte character, so on a profile path like `C:\Users\José` the
    // registered `--log-file` and `--config-dir` arrive mojibake'd: the agent
    // logs into a directory that does not exist, and a corrupted `--config-dir`
    // mints a fresh identity and rejoins the fleet as a stranger.
    //
    // `-LiteralPath` for the same class of reason: `-Path` globs, and
    // `C:\Users\[lab]\...` is a legal path whose brackets `-Path` reads as a
    // character class, failing a perfectly valid install with "cannot find
    // path".
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
                "Register-ScheduledTask -TaskName '{SERVICE_NAME}'                  -Xml (Get-Content -Raw -Encoding UTF8 -LiteralPath {quoted_unit}) \
                 -Force | Out-Null"
            )),
            ps(&format!("Start-ScheduledTask -TaskName '{SERVICE_NAME}'")),
        ],
        // The same question `is-active` answers: the task EXISTING is not the
        // agent running, and `Get-ScheduledTask` alone would report success for
        // a task whose process died on startup.
        // Polled, then CONFIRMED. `Start-ScheduledTask` returns as soon as the
        // scheduler has accepted the request, not when the process is up, so a
        // single read right after it reports `Ready` and the install announces
        // "installed, but it is NOT running" for an agent that is about to be.
        // That is what a REINSTALL did on CI, because a replaced task has to be
        // stopped and started rather than merely started.
        //
        // But waiting for ANY `Running` observation is weaker than one read:
        // an agent that dies after a second would be seen alive on the way
        // past. So the first sighting only starts a second look a second
        // later, and both have to agree.
        //
        // THIRTY seconds, not ten. Ten was enough on a warm machine and not on
        // a cold CI runner, where the agent has to start, read its config dir,
        // load a token and probe docker before it reports Running -- and where
        // probing for a docker that is not there is itself slow. The install
        // then announced "installed, but it is NOT running" for an agent that
        // was merely still starting, which is the exact false alarm this poll
        // was written to prevent. The only cost of the larger deadline is that a
        // genuinely dead agent takes longer to be called dead.
        verify: ps(&format!(
            "$deadline = (Get-Date).AddSeconds(30); \
             $running = {{ (Get-ScheduledTask -TaskName '{SERVICE_NAME}' \
                 -ErrorAction SilentlyContinue).State -eq 'Running' }}; \
             do {{ \
               if (& $running) {{ \
                 Start-Sleep -Seconds 1; \
                 if (& $running) {{ exit 0 }} \
               }}; \
               Start-Sleep -Milliseconds 250 \
             }} while ((Get-Date) -lt $deadline); \
             exit 1"
        )),
        best_effort: Vec::new(),
        deactivate: vec![
            // STOP first. Unregistering a task does not end its running
            // instance -- the same fact `pre_activate` above exists for -- so
            // uninstall printed "agent service removed" while the old agent kept
            // holding the browser and peer ports until logoff, still serving its
            // token and its pins. The user then cannot even start a new one:
            // `agent run` refuses the busy port, and the supervisor that could
            // have stopped it is the thing they just removed.
            //
            // `SilentlyContinue`: a task that is registered but not running is
            // the normal case for an uninstall after a reboot, and failing there
            // would leave the task registered forever.
            ps(&format!(
                "Stop-ScheduledTask -TaskName '{SERVICE_NAME}' -ErrorAction SilentlyContinue"
            )),
            ps(&format!(
                "Unregister-ScheduledTask -TaskName '{SERVICE_NAME}' -Confirm:$false"
            )),
        ],
    }
}

/// Quote one argument the way `CommandLineToArgvW` parses it.
///
/// Not the same rules as a shell: a backslash is literal EXCEPT immediately
/// before a quote, where it escapes. Getting this wrong turns
/// `C:\Users\me\bin\` into an unterminated quote and the whole command line
/// into one unusable argument.
#[cfg(test)]
pub(crate) fn windows_quote_for_test(a: &str) -> String {
    windows_quote(a)
}

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
                // The run is ALREADY in `out` once, so this adds n more to
                // double it, plus one to escape the quote. Extending by
                // `2n + 1` emitted `3n + 1` and CommandLineToArgvW parsed
                // `a\"b` back as `a\\b` — the quote silently gone.
                out.extend(std::iter::repeat_n('\\', backslashes + 1));
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
