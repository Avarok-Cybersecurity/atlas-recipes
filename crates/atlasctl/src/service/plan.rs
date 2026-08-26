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
}

impl ServiceKind {
    /// What this build targets.
    ///
    /// Windows is deliberately absent rather than guessed at: it needs a
    /// Scheduled Task at logon, not a service, because session 0 cannot reach
    /// Docker Desktop's per-user named pipe.
    ///
    /// # Errors
    /// On a platform with no supported supervisor.
    pub fn detect() -> Result<Self> {
        if cfg!(target_os = "linux") {
            Ok(Self::Systemd)
        } else if cfg!(target_os = "macos") {
            Ok(Self::Launchd)
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
}

impl AgentInvocation {
    /// The argv the supervisor will execute.
    ///
    /// The port is always written out even when it matches the default. A unit
    /// file outlives the binary that wrote it, so a later change to the default
    /// must not silently move an installed service to a different port.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        let mut v = vec![
            self.exe.display().to_string(),
            "agent".to_owned(),
            "run".to_owned(),
            "--port".to_owned(),
            self.port.to_string(),
        ];
        if self.client {
            v.push("--client".to_owned());
        }
        if !self.discovery {
            v.push("--no-discovery".to_owned());
        }
        if !self.browser {
            v.push("--no-browser".to_owned());
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
    /// Commands to run after writing it, in order.
    pub activate: Vec<Vec<String>>,
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
        activate: vec![sc(&["daemon-reload"]), sc(&["enable", "--now", &unit])],
        // A headless box logs nobody in, so without lingering the service stops
        // the moment the installing session ends. It is also exactly the call
        // that is unavailable in a container, so it cannot be required.
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
        activate: vec![vec![
            "launchctl".to_owned(),
            "bootstrap".to_owned(),
            target.clone(),
            path,
        ]],
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
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
