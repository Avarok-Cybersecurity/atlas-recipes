// SPDX-License-Identifier: AGPL-3.0-only

//! The orderings and the quoting, which are what actually break.
//!
//! No systemd and no filesystem: both are injected, so every case below is the
//! real code path with recording doubles in place of the machine.

use super::plan::{AgentInvocation, ServiceKind, plan};
use super::*;
use anyhow::Result;
use atlasctl_core::io::process::Output;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Records every argv, and can be told which ones fail.
struct Recorder {
    calls: Mutex<Vec<String>>,
    fail: Vec<String>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: Vec::new(),
        }
    }
    fn failing(what: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: vec![what.to_owned()],
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("lock").clone()
    }
}

impl ProcessRunner for Recorder {
    fn run(&self, argv: &[String]) -> Result<Output> {
        let line = argv.join(" ");
        self.calls.lock().expect("lock").push(line.clone());
        let failed = self.fail.iter().any(|f| line.contains(f.as_str()));
        Ok(Output {
            status: i32::from(failed),
            stdout: String::new(),
            stderr: if failed {
                "Interactive authentication required.\n".to_owned()
            } else {
                String::new()
            },
        })
    }
    fn run_streaming(&self, _: &[String]) -> Result<i32> {
        unreachable!("the installer never streams")
    }
}

/// A filesystem that records writes and removals.
#[derive(Default)]
struct Files {
    written: Mutex<Vec<(PathBuf, String)>>,
    removed: Mutex<Vec<PathBuf>>,
    dirs: Mutex<Vec<PathBuf>>,
}

impl Files {
    fn body(&self) -> String {
        self.written
            .lock()
            .expect("lock")
            .first()
            .map(|(_, b)| b.clone())
            .unwrap_or_default()
    }
}

impl FileSystem for Files {
    fn read_to_string(&self, _: &Path) -> Result<String> {
        anyhow::bail!("the installer never reads")
    }
    fn write_atomic(&self, path: &Path, contents: &str) -> Result<()> {
        self.written
            .lock()
            .expect("lock")
            .push((path.to_path_buf(), contents.to_owned()));
        Ok(())
    }
    fn exists(&self, _: &Path) -> bool {
        false
    }
    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.dirs.lock().expect("lock").push(path.to_path_buf());
        Ok(())
    }
    fn list_files(&self, _: &Path, _: &str) -> Result<Vec<PathBuf>> {
        Ok(Vec::new())
    }
    fn remove_file(&self, path: &Path) -> Result<()> {
        self.removed.lock().expect("lock").push(path.to_path_buf());
        Ok(())
    }
}

fn agent() -> AgentInvocation {
    AgentInvocation {
        exe: PathBuf::from("/home/o/.local/bin/atlasctl"),
        port: 34333,
        client: false,
        discovery: true,
        browser: true,
        config_dir: None,
    }
}

fn home() -> PathBuf {
    PathBuf::from("/home/o")
}

// ---- the pure plan -------------------------------------------------------

/// systemd refuses to enable a unit it has not read. Enabling before the
/// reload fails in a way that reads like the unit was never written at all.
#[test]
fn systemd_reloads_before_it_enables() {
    let p = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    let order: Vec<String> = p.activate.iter().map(|a| a.join(" ")).collect();
    let reload = order.iter().position(|c| c.contains("daemon-reload"));
    let enable = order.iter().position(|c| c.contains("enable"));
    assert!(reload < enable, "reload must come first: {order:?}");
}

/// Reinstalling must actually restart the agent.
///
/// `enable --now` starts a stopped unit and does nothing to a running one, so
/// on a reinstall the agent keeps the flags it was first installed with — a
/// different port, a stale config dir, browser mode when `--no-browser` was
/// asked for — while the command reports success. The operator then pairs
/// against a process that is not the one they just configured.
#[test]
fn reinstalling_restarts_rather_than_leaving_the_old_agent_running() {
    let p = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    let order: Vec<String> = p.activate.iter().map(|a| a.join(" ")).collect();
    let enable = order
        .iter()
        .position(|c| c.contains("enable"))
        .expect("the unit is enabled");
    let restart = order
        .iter()
        .position(|c| c.contains("restart"))
        .expect("a reinstall must restart the unit, not assume --now did it");
    assert!(
        enable < restart,
        "enable must precede restart, or the first install restarts a unit that \
         is not enabled yet: {order:?}"
    );
}

/// The port is written even when it equals today's default. A unit file
/// outlives the binary that wrote it, so a later change to the default must
/// not silently move an installed service to a different port.
#[test]
fn the_unit_pins_the_port_explicitly() {
    let p = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    assert!(
        p.unit_body.contains("--port 34333"),
        "the unit did not pin the port:\n{}",
        p.unit_body
    );
}

/// A control-only install must stay control-only across a reboot. Dropping the
/// flag would silently promote a laptop into a machine that runs models.
#[test]
fn a_control_only_install_carries_the_flag_into_the_unit() {
    let a = AgentInvocation {
        client: true,
        ..agent()
    };
    let p = plan(ServiceKind::Systemd, &a, &home(), 1000);
    assert!(p.unit_body.contains("--client"), "{}", p.unit_body);

    let normal = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    assert!(!normal.unit_body.contains("--client"));
}

/// A node installed to hold a rank must still be one after a reboot. Dropping
/// the flag would have it come back demanding a browser credential it does not
/// use — the failure that stopped a worker starting at all.
#[test]
fn a_rank_holder_install_records_no_browser() {
    let a = AgentInvocation {
        browser: false,
        ..agent()
    };
    let p = plan(ServiceKind::Systemd, &a, &home(), 1000);
    assert!(p.unit_body.contains("--no-browser"), "{}", p.unit_body);

    let normal = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    assert!(
        !normal.unit_body.contains("--no-browser"),
        "an ordinary install must still serve a browser"
    );
}

/// systemd splits ExecStart on whitespace itself, so an unquoted path with a
/// space in it invokes a different binary — or nothing.
#[test]
fn a_path_with_a_space_is_quoted_in_execstart() {
    let a = AgentInvocation {
        exe: PathBuf::from("/home/My User/bin/atlasctl"),
        ..agent()
    };
    let p = plan(ServiceKind::Systemd, &a, &home(), 1000);
    let exec = p
        .unit_body
        .lines()
        .find(|l| l.starts_with("ExecStart="))
        .expect("ExecStart");
    assert!(
        exec.contains("\"/home/My User/bin/atlasctl\""),
        "unquoted path in {exec}"
    );
}

/// The plist is XML, so an argument carrying a metacharacter must not be able
/// to close a tag and rewrite the rest of the document.
#[test]
fn plist_arguments_are_xml_escaped() {
    let a = AgentInvocation {
        exe: PathBuf::from("/o/<a&b>/atlasctl"),
        ..agent()
    };
    let p = plan(ServiceKind::Launchd, &a, &home(), 501);
    assert!(p.unit_body.contains("&lt;a&amp;b&gt;"), "{}", p.unit_body);
    assert!(
        !p.unit_body.contains("/o/<a&b>/"),
        "raw metacharacters reached the plist"
    );
}

/// linger is what keeps the agent alive on a headless box after the installing
/// session ends — and it is exactly the call that is unavailable in a
/// container, so it can never be a required step.
#[test]
fn lingering_is_best_effort_not_required() {
    let p = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    let required: Vec<String> = p.activate.iter().map(|a| a.join(" ")).collect();
    assert!(
        !required.iter().any(|c| c.contains("enable-linger")),
        "linger must not be able to fail an install: {required:?}"
    );
    assert!(
        p.best_effort
            .iter()
            .any(|a| a.join(" ").contains("enable-linger"))
    );
}

// ---- the I/O half --------------------------------------------------------

#[cfg(target_os = "linux")]
#[test]
fn a_successful_install_writes_the_unit_then_activates_it() {
    let fs = Files::default();
    let r = Recorder::new();
    let out = install(&fs, &r, &agent(), &home(), 1000).expect("installs");

    assert_eq!(
        out.unit_path,
        home().join(".config/systemd/user/atlasctl-agent.service")
    );
    assert!(fs.body().contains("ExecStart="));
    assert!(out.skipped.is_empty());
    let calls = r.calls();
    assert!(calls[0].contains("daemon-reload"), "{calls:?}");
    assert!(calls[1].contains("enable --now"), "{calls:?}");
}

/// A failed linger is reported, not swallowed: on a headless machine it is the
/// difference between an agent that survives logout and one that does not, and
/// the operator can only act on it if they are told.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_best_effort_step_is_reported_but_still_installs() {
    let fs = Files::default();
    let r = Recorder::failing("enable-linger");
    let out = install(&fs, &r, &agent(), &home(), 1000).expect("still installs");
    assert_eq!(out.skipped.len(), 1);
    assert!(
        out.skipped[0].contains("enable-linger"),
        "{:?}",
        out.skipped
    );
    assert!(
        out.skipped[0].contains("Interactive authentication required"),
        "the reason must survive: {:?}",
        out.skipped
    );
}

/// A required step failing must fail the install, and must say which step.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_required_step_fails_the_install_and_names_itself() {
    let fs = Files::default();
    let r = Recorder::failing("enable --now");
    let e = install(&fs, &r, &agent(), &home(), 1000).expect_err("must fail");
    let msg = format!("{e:#}");
    assert!(msg.contains("enable --now"), "{msg}");
    // The unit stays on disk: it is the evidence for why the step failed.
    assert_eq!(fs.written.lock().expect("lock").len(), 1);
}

/// A supervisor asked to disable a unit whose file has already gone can
/// refuse, leaving the service running with nothing on disk to stop it with.
#[cfg(target_os = "linux")]
#[test]
fn uninstall_stops_the_service_before_deleting_its_unit() {
    let fs = Files::default();
    let r = Recorder::new();
    let path = uninstall(&fs, &r, &home(), 1000).expect("uninstalls");

    assert_eq!(
        path,
        home().join(".config/systemd/user/atlasctl-agent.service")
    );
    assert!(r.calls()[0].contains("disable --now"), "{:?}", r.calls());
    assert_eq!(*fs.removed.lock().expect("lock"), vec![path]);
}

/// Uninstalling twice, or uninstalling something a half-finished install never
/// wrote, is the ordinary case and must not be an error.
#[cfg(target_os = "linux")]
#[test]
fn uninstalling_something_that_was_never_installed_succeeds() {
    let fs = Files::default();
    let r = Recorder::failing("disable");
    uninstall(&fs, &r, &home(), 1000).expect("must not fail");
}

/// A relocated config dir must reach the unit, or the service starts against
/// the default one.
///
/// This is the failure that looks like a security problem and is not: the
/// install joins the fleet using `/data/atlas`, the unit runs `agent run` with
/// no config dir, the service finds no `agent.key` where it looked, mints a
/// fresh identity, and the node the operator paired sixty seconds ago comes
/// back as an unknown machine with no pins.
#[test]
fn a_relocated_config_dir_is_recorded_in_the_unit() {
    let mut a = agent();
    a.config_dir = Some(PathBuf::from("/data/atlas"));
    let argv = a.argv();
    let joined = argv.join(" ");
    assert!(
        joined.contains("--config-dir /data/atlas"),
        "the unit must carry the config dir: {joined}"
    );
    // Global flags bind to the top-level command, so it has to precede the
    // subcommand or the unit is not a command anyone could have typed.
    let flag = argv
        .iter()
        .position(|s| s == "--config-dir")
        .expect("present");
    let sub = argv.iter().position(|s| s == "agent").expect("present");
    assert!(
        flag < sub,
        "--config-dir is global and must precede `agent`: {argv:?}"
    );
}

/// And the default must NOT be written out. A resolved default baked into a
/// unit outlives the default it came from.
#[test]
fn an_unset_config_dir_writes_no_flag() {
    let joined = agent().argv().join(" ");
    assert!(
        !joined.contains("--config-dir"),
        "no flag when the operator chose none: {joined}"
    );
}
