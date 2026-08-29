// SPDX-License-Identifier: AGPL-3.0-only

//! What installing the agent as a background service consists of.
//!
//! Pure. Rendering the unit file and choosing the argv sequences happens here,
//! with no filesystem and no processes, because that is the part with rules in
//! it — the paths, the flags the service is started with, the order the
//! commands have to run in. [`super`] does the I/O.
//!
//! Splitting it this way is what makes the interesting failure testable: an
//! install that writes a unit naming the wrong binary, or one that enables a
//! service before reloading the daemon that would notice it exists.

mod windows;

pub use windows::windows_log_path;
#[cfg(test)]
pub(crate) use windows::windows_quote_for_test;

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// The name the service is known by on both platforms.
pub const SERVICE_NAME: &str = "atlasctl-agent";

/// Reverse-DNS label launchd wants.
pub const LAUNCHD_LABEL: &str = "io.atlasinference.atlasctl-agent";

/// How this machine supervises background processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// `systemd --user`, on Linux.
    Systemd,
    /// A launchd LaunchAgent, on macOS.
    Launchd,
    /// A Task Scheduler task at logon, on Windows.
    ///
    /// A task and not a Windows service, and at logon rather than at boot: a
    /// service runs in session 0, which cannot reach Docker Desktop's per-user
    /// named pipe, so the agent would come up healthy and be unable to launch
    /// anything.
    ScheduledTask,
}

impl ServiceKind {
    /// What this build targets.
    ///
    /// # Errors
    /// On a platform with no supported supervisor.
    pub fn detect() -> Result<Self> {
        if cfg!(target_os = "linux") {
            Ok(Self::Systemd)
        } else if cfg!(target_os = "macos") {
            Ok(Self::Launchd)
        } else if cfg!(target_os = "windows") {
            Ok(Self::ScheduledTask)
        } else {
            bail!(
                "installing a background service is not supported on this platform yet.\n\
                 Run `atlasctl agent run` and keep the terminal open, or supervise it \
                 with whatever this machine already uses."
            )
        }
    }
}

/// How the agent should be started when the service brings it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInvocation {
    /// Absolute path to the atlasctl binary.
    pub exe: PathBuf,
    /// Port the browser channel listens on.
    pub port: u16,
    /// Whether this machine runs as a control node only.
    pub client: bool,
    /// Whether to advertise on, and listen to, the network.
    pub discovery: bool,
    /// Whether to serve the browser channel at all.
    ///
    /// Recorded in the unit like the rest: a node installed as a rank-holder
    /// must still be one after a reboot, rather than quietly starting to
    /// demand a browser credential it does not use.
    pub browser: bool,
    /// Where this agent keeps its state, when it is not the default.
    ///
    /// Recorded for the same reason the port is, only the stakes are higher.
    /// `--config-dir` is a global flag, so `atlasctl --config-dir /data agent
    /// install --join …` performs the join against `/data` — and then wrote a
    /// unit that ran `agent run` with no config dir at all. The service came up
    /// against the default directory, found no `agent.key`, minted a NEW
    /// identity, and rejoined the fleet as a stranger with zero pins. The
    /// operator sees a node that paired a moment ago and is now untrusted.
    pub config_dir: Option<PathBuf>,
    /// Where the supervised agent's output is appended, when the supervisor
    /// does not capture it itself.
    ///
    /// Recorded in the unit for the same reason the port is: a unit outlives
    /// the binary that wrote it, and an agent whose log moved without its unit
    /// knowing writes where nobody is looking.
    pub log_file: Option<PathBuf>,
}

impl AgentInvocation {
    /// The argv the supervisor will execute.
    ///
    /// The port is always written out even when it matches the default. A unit
    /// file outlives the binary that wrote it, so a later change to the default
    /// must not silently move an installed service to a different port.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let mut v = vec![self.exe.display().to_string()];
        // Before the subcommand: it is a global flag, and putting it here means
        // the unit reads the way the operator would type it.
        if let Some(dir) = &self.config_dir {
            v.push("--config-dir".to_owned());
            v.push(dir.display().to_string());
        }
        v.extend([
            "agent".to_owned(),
            "run".to_owned(),
            "--port".to_owned(),
            self.port.to_string(),
        ]);
        if self.client {
            v.push("--client".to_owned());
        }
        if !self.discovery {
            v.push("--no-discovery".to_owned());
        }
        if !self.browser {
            v.push("--no-browser".to_owned());
        }
        if let Some(log) = &self.log_file {
            v.push("--log-file".to_owned());
            v.push(log.display().to_string());
        }
        v
    }
}

/// Everything an install or uninstall has to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePlan {
    /// Which supervisor.
    pub kind: ServiceKind,
    /// Where the unit or plist is written.
    pub unit_path: PathBuf,
    /// Its contents.
    pub unit_body: String,
    /// Commands to run BEFORE activation, whose failure is expected and
    /// ignored, in order.
    ///
    /// This exists for one shape: making activation idempotent. `launchctl
    /// bootstrap` refuses a label that is already loaded, with
    /// `Bootstrap failed: 5: Input/output error` — a message that names neither
    /// the cause nor the remedy — so re-running the installer to UPGRADE an
    /// agent failed at the last step, and `atlasctl agent install`, which the
    /// installer suggests as the way to see why, reproduced the same line.
    ///
    /// Separate from `best_effort` because ORDER is the whole point: a teardown
    /// that runs after the thing it was meant to make room for is useless.
    pub pre_activate: Vec<Vec<String>>,
    /// Commands to run after writing it, in order.
    pub activate: Vec<Vec<String>>,
    /// The command that answers "is it actually running now?".
    ///
    /// Activation succeeding only means the supervisor accepted the unit. An
    /// agent that exits immediately — a config dir it cannot write, a port
    /// already taken — is restarted every RestartSec forever, and `enable
    /// --now` still returned 0. Without this the install prints "installed and
    /// started" and sends the operator off to pair a browser against a process
    /// that never lived.
    pub verify: Vec<String>,
    /// Commands whose failure is expected and must not fail the install.
    ///
    /// Separated rather than flagged inline: an install that reports success
    /// after a step that actually failed is worse than one that never ran the
    /// step, so "may fail" has to be a property of the plan a reader can see.
    pub best_effort: Vec<Vec<String>>,
    /// Commands to run before removing it, in order.
    pub deactivate: Vec<Vec<String>>,
}

/// Plan an install for this machine.
///
/// `home` and `uid` are passed rather than read so this stays pure; the caller
/// owns discovering them.
///
/// # Errors
/// If the platform has no supported supervisor.
pub fn plan(kind: ServiceKind, agent: &AgentInvocation, home: &Path, uid: u32) -> ServicePlan {
    match kind {
        ServiceKind::Systemd => systemd(agent, home),
        ServiceKind::Launchd => launchd(agent, home, uid),
        ServiceKind::ScheduledTask => windows::scheduled_task(agent, home),
    }
}

fn systemd(agent: &AgentInvocation, home: &Path) -> ServicePlan {
    let unit = format!("{SERVICE_NAME}.service");
    let unit_path = home.join(".config/systemd/user").join(&unit);
    let exec = shell_words(&agent.argv());

    // MemoryMax, not MemoryHigh: this is a forever-process with docker access,
    // and a hard ceiling that kills it is easier to notice than a soft one that
    // silently throttles it into looking hung.
    let unit_body = format!(
        "# Written by `atlasctl agent install`. Edits here are NOT kept:\n\
         # reinstalling overwrites this file. Put local changes in a drop-in\n\
         # under {SERVICE_NAME}.service.d/ instead.\n\
         [Unit]\n\
         Description=Atlas agent — the local control plane the website talks to\n\
         Documentation=https://atlasinference.io/control.html\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         MemoryMax=256M\n\
         # The agent re-adopts containers it already started, so a restart does\n\
         # not orphan a running model.\n\
         KillMode=mixed\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    );

    let sc = |args: &[&str]| {
        let mut v = vec!["systemctl".to_owned(), "--user".to_owned()];
        v.extend(args.iter().map(|s| (*s).to_owned()));
        v
    };

    ServicePlan {
        kind: ServiceKind::Systemd,
        unit_path,
        unit_body,
        // daemon-reload first: enabling a unit systemd has not read yet fails,
        // and it fails in a way that reads like the unit was never written.
        //
        // `restart` after `enable --now` is not redundant. On a first install
        // `--now` starts the unit and the restart is a no-op. On a REinstall
        // the unit is already active, so `--now` does nothing at all and the
        // agent keeps running with the flags it was installed with — a
        // different port, a stale --config-dir, browser mode when the operator
        // asked for --no-browser — while the command prints "agent installed
        // and started". Silently succeeding while doing nothing is worse than
        // failing, because the operator goes off and pairs against the old one.
        activate: vec![
            sc(&["daemon-reload"]),
            sc(&["enable", "--now", &unit]),
            sc(&["restart", &unit]),
        ],
        // A shell, deliberately, and consistent with the Windows plan two
        // functions down, which hands PowerShell a script the same way. The
        // runner's "argv, never a command string" property is about the RUNNER
        // not interpreting what it is given -- it still passes three inert
        // arguments -- and everything interpolated below is a compile-time
        // constant. Doing the wait in Rust instead would put a three-second
        // sleep in the install path's unit tests, and the only way around that
        // is an injection point that exists for tests, which this repo does not
        // allow in a production path.
        //
        // Delayed, then CONFIRMED, for the reason the Windows plan spells out.
        // `is-active` ran the instant `restart` returned, and for a Type=simple
        // unit that is "active" as soon as ExecStart is SPAWNED -- while the
        // agent has yet to check its config dir, load its token, probe docker or
        // bind its port. Every failure this check exists to catch (a directory it
        // cannot write, a port already taken) happens after that read, so the
        // install printed "agent installed and started" and the operator went off
        // to pair a browser against a five-second crash loop.
        //
        // Two seconds covers the agent's startup; the second look one second
        // later is what distinguishes "up" from "up so far" -- a single delayed
        // read would still bless a unit on its way down.
        verify: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!(
                "sleep 2; systemctl --user is-active --quiet {unit} || exit 1; \
                 sleep 1; systemctl --user is-active --quiet {unit}"
            ),
        ],
        // A headless box logs nobody in, so without lingering the service stops
        // the moment the installing session ends. It is also exactly the call
        // that is unavailable in a container, so it cannot be required.
        // systemd needs none: `enable --now` is already idempotent, and
        // re-running it over a live unit restarts it with the new binary.
        pre_activate: Vec::new(),
        best_effort: vec![vec!["loginctl".to_owned(), "enable-linger".to_owned()]],
        deactivate: vec![sc(&["disable", "--now", &unit])],
    }
}

fn launchd(agent: &AgentInvocation, home: &Path, uid: u32) -> ServicePlan {
    let unit_path = home
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"));
    let args = agent
        .argv()
        .iter()
        .map(|a| format!("    <string>{}</string>", xml_escape(a)))
        .collect::<Vec<_>>()
        .join("\n");
    let logs = home
        .join("Library/Logs")
        .join(format!("{SERVICE_NAME}.log"));
    let log = xml_escape(&logs.display().to_string());

    let unit_body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>Label</key>\n\
         \x20 <string>{LAUNCHD_LABEL}</string>\n\
         \x20 <key>ProgramArguments</key>\n\
         \x20 <array>\n{args}\n\
         \x20 </array>\n\
         \x20 <key>RunAtLoad</key>\n\
         \x20 <true/>\n\
         \x20 <key>KeepAlive</key>\n\
         \x20 <dict><key>SuccessfulExit</key><false/></dict>\n\
         \x20 <key>StandardOutPath</key>\n\
         \x20 <string>{log}</string>\n\
         \x20 <key>StandardErrorPath</key>\n\
         \x20 <string>{log}</string>\n\
         </dict>\n\
         </plist>\n"
    );

    let target = format!("gui/{uid}");
    let path = unit_path.display().to_string();
    ServicePlan {
        kind: ServiceKind::Launchd,
        unit_path: unit_path.clone(),
        unit_body,
        // Unload first, so installing over an existing agent UPGRADES it
        // rather than failing. `bootout` exits non-zero when the label is not
        // loaded, which is the ordinary first-install case, so it cannot be an
        // `activate` step.
        pre_activate: vec![vec![
            "launchctl".to_owned(),
            "bootout".to_owned(),
            format!("{target}/{LAUNCHD_LABEL}"),
        ]],
        activate: vec![vec![
            "launchctl".to_owned(),
            "bootstrap".to_owned(),
            target.clone(),
            path,
        ]],
        // `launchctl print` exits non-zero when the label is not loaded, which
        // is the same question `is-active` answers on systemd.
        //
        // ⚠ Weaker than the Linux and Windows checks, knowingly. `print` exits 0
        // whenever the label is LOADED, so a KeepAlive crash loop -- an agent
        // dying and being restarted every second -- reads as healthy, and the
        // "installed, but it is NOT running" branch cannot fire on macOS. The
        // check that would work is `state = running` from this same output (or a
        // `pid = ` line), and it is not made here because a wrong verify string
        // would fail the install for every Mac user and there is no macOS runner
        // in this repo to prove it against. Sleep first so at least an agent that
        // exits before launchd even records it is not blessed.
        verify: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            format!("sleep 2; launchctl print {target}/{LAUNCHD_LABEL}"),
        ],
        best_effort: Vec::new(),
        deactivate: vec![vec![
            "launchctl".to_owned(),
            "bootout".to_owned(),
            format!("{target}/{LAUNCHD_LABEL}"),
        ]],
    }
}

/// Quote an argv for a systemd `ExecStart=` line.
///
/// systemd splits `ExecStart` on whitespace itself, so a path containing a
/// space has to be quoted or the unit silently invokes the wrong binary.
fn shell_words(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() || a.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\') {
                format!("\"{}\"", a.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Escape a value for the plist, which is XML.
pub(super) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
