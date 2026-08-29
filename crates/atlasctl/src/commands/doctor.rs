// SPDX-License-Identifier: AGPL-3.0-only

//! `doctor` — check this machine for problems.

use anyhow::Result;
use atlasctl_core::io::{ProcessRunner, StdProcessRunner};

use super::doctor_checks::{self, ConfigDirState, Finding};

/// The compromised registry that sparkrun redirects to.
const COMPROMISED_HOST: &str = "Atlas-Inf/sparkrun-recipes";

/// Run every check and report.
pub fn run() -> Result<()> {
    let mut problems = 0;

    problems += check_docker();
    // The three that were added after the fact, each one a failure somebody hit
    // during onboarding and diagnosed from an error naming the wrong thing.
    for f in [
        check_config_dir(),
        check_agent(),
        check_reachable(),
        check_disk(),
    ] {
        println!("{}", f.line);
        problems += usize::from(f.problem);
    }
    problems += check_sparkrun();

    println!();
    if problems == 0 {
        println!("no problems found");
        return Ok(());
    }

    // A diagnostic that always exits 0 cannot be gated on. SECURITY.md tells an
    // operator to run this to find a compromised sparkrun install -- a registry
    // redirect that lets someone else's recipes run shell commands on this host
    // -- and a script wrapping that check had no way to see the answer, because
    // the report went to stdout and the status was always success. Reporting a
    // finding is not the same as failing to run, but every tool that gates on
    // this one can only read the status, so `brew doctor`'s convention applies:
    // print the report, then exit non-zero.
    Err(anyhow::anyhow!(
        "{problems} problem(s) found — see the report above"
    ))
}

fn check_docker() -> usize {
    match StdProcessRunner.run(&[
        "docker".into(),
        "version".into(),
        "--format".into(),
        "{{.Server.Version}}".into(),
    ]) {
        Ok(out) if out.success() => {
            println!("docker:   ok (server {})", out.stdout.trim());
            0
        }
        Ok(_) => {
            println!(
                "docker:   PROBLEM — the docker CLI is present but the daemon did not answer.\n\
                 \x20         `atlasctl run` needs a working daemon; `recipe list` and\n\
                 \x20         `run --print` work without one."
            );
            1
        }
        Err(_) => {
            println!(
                "docker:   PROBLEM — no docker on PATH. `atlasctl run` needs docker and the\n\
                 \x20         NVIDIA container runtime; inspection commands work without them."
            );
            1
        }
    }
}

/// Look for a sparkrun install whose registry has been redirected.
///
/// This is why `doctor` exists. sparkrun 0.3.6 rewrites the Atlas registry URL
/// to a repository under an organisation Atlas does not control, and marks it
/// trusted — which lets recipe-supplied shell commands run on the host. We
/// report it and print the exact removal commands. We never delete a user's
/// files: that behaviour is precisely what makes a tool untrustworthy.
fn check_sparkrun() -> usize {
    let Ok(home) = std::env::var("HOME") else {
        return 0;
    };
    let config = std::path::Path::new(&home).join(".config/sparkrun/registries.yaml");
    let installed = atlasctl_core::platform::which("sparkrun").is_some();
    let redirected = std::fs::read_to_string(&config)
        .map(|s| s.contains(COMPROMISED_HOST))
        .unwrap_or(false);

    if !installed && !redirected {
        println!("sparkrun: not installed");
        return 0;
    }

    println!("sparkrun: PROBLEM — a sparkrun install was found.");
    if redirected {
        println!(
            "\x20         Its config at {} points the `atlas` registry at",
            config.display()
        );
        println!("\x20         {COMPROMISED_HOST}, which Atlas does not control.");
        println!(
            "\x20         Editing the file is not enough: the redirect is compiled into\n\
             \x20         sparkrun, so it is reapplied the next time the tool runs."
        );
    }
    println!("\x20         A trusted registry's recipes can run shell commands on this host.");
    println!("\x20         To remove it:");
    println!("\x20           pipx uninstall sparkrun     # or: uv tool uninstall sparkrun");
    println!("\x20           rm -rf ~/.config/sparkrun ~/.cache/sparkrun");
    println!("\x20         Review those directories first; atlasctl will not delete them for you.");
    1
}

/// Free space where docker and the model cache live.
///
/// Measured at the HF cache, not at the current directory. It used to read
/// `Path::new(".")`, which answers about whatever volume the operator happened
/// to `cd` into: on a box whose models live on a separate mount, doctor printed
/// `disk: ok` while `run` immediately warned, and run from a small partition it
/// failed a healthy machine -- which `disk_unknown`'s own doc says must not
/// happen. `run` has always measured `host.hf_cache_dir` (run.rs:96); this now
/// asks the same question of the same volume.
///
/// Falls back to `.` when the cache directory does not exist yet, which is the
/// ordinary state of a machine that has never pulled a model. Reporting
/// "unknown" there would be a regression from a usable answer to none.
///
/// Read with `df -Pk`: POSIX output is one line per filesystem with a fixed
/// column order, which `df` without `-P` does not guarantee — a long device
/// name wraps and the figure moves to the next line.
///
/// A machine we cannot measure is reported as ok rather than as a problem: an
/// unreadable `df` says nothing about whether there is room, and doctor now
/// exits non-zero on a problem, so guessing here would fail a healthy box.
fn check_disk() -> Finding {
    let cache = crate::hostinfo::snapshot().ok().map(|h| h.hf_cache_dir);
    let at_cache = cache
        .as_deref()
        .and_then(|d| atlasctl_core::platform::free_bytes(std::path::Path::new(d)));
    let at_cwd = atlasctl_core::platform::free_bytes(std::path::Path::new("."));
    disk_finding(cache.as_deref().zip(at_cache), at_cwd)
}

/// Which measurement to report, given what could be read.
///
/// Split from the I/O so the ORDER is testable: preferring the cache is the
/// whole point of the fix, and falling back to `.` rather than to "unknown"
/// is what keeps a machine that has never pulled a model from losing a usable
/// answer. Both are one-line mistakes to make and invisible without a test.
fn disk_finding(at_cache: Option<(&str, u64)>, at_cwd: Option<u64>) -> Finding {
    match (at_cache, at_cwd) {
        (Some((dir, bytes)), _) => doctor_checks::disk_space(bytes, dir),
        (None, Some(bytes)) => doctor_checks::disk_space(bytes, "."),
        (None, None) => doctor_checks::disk_unknown(),
    }
}

/// Where the agent keeps its identity, its pins and its browser token.
fn check_config_dir() -> Finding {
    match crate::hostinfo::usable_config_dir() {
        Ok(dir) => doctor_checks::config_dir(ConfigDirState::Writable(dir.display().to_string())),
        // `usable_config_dir` already distinguishes "cannot create" from "not
        // writable" and says so; doctor reuses that judgement rather than
        // re-deriving it and risking a second, disagreeing opinion.
        Err(e) => doctor_checks::config_dir(ConfigDirState::Unusable(format!("{e:#}"))),
    }
}

/// Whether this machine's own agent is answering.
fn check_agent() -> Finding {
    let port = atlasctl_agent::DEFAULT_PORT;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let up =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok();
    doctor_checks::agent(up, port)
}

/// Whether another machine could reach this one.
fn check_reachable() -> Finding {
    use atlasctl_agent::fabric::FabricProvider as _;
    // Same provider selection as `agent run`, so doctor answers about the
    // interfaces the agent would actually enumerate rather than a second
    // opinion that could disagree with it.
    #[cfg(target_os = "macos")]
    let fabric = atlasctl_agent::fabric::macos::MacFabric::new();
    #[cfg(not(target_os = "macos"))]
    let fabric = atlasctl_agent::fabric::linux::LinuxFabric::new();

    // NOT `unwrap_or_default()`. An enumeration that failed is not a machine
    // with no addresses, and the advice for the two is unrelated — telling
    // someone to plug in a network cable because `/sys/class/net` could not be
    // listed sends them to fix the wrong thing.
    match fabric.addresses() {
        Ok(list) => {
            let addrs = list
                .into_iter()
                .map(|a| (a.iface, a.addr))
                .collect::<Vec<_>>();
            doctor_checks::reachable(&addrs)
        }
        Err(e) => doctor_checks::unreadable_interfaces(&format!("{e:#}")),
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;

    const PLENTY: u64 = 500 * 1024 * 1024 * 1024;

    /// The cache volume wins when it can be read, and the line SAYS which
    /// volume it measured — the old code reported `.` and meant it.
    #[test]
    fn the_model_cache_is_preferred_and_named() {
        let f = disk_finding(Some(("/mnt/models", PLENTY)), Some(1));
        assert!(f.line.contains("/mnt/models"), "got: {}", f.line);
        assert!(!f.line.contains(" on ."), "got: {}", f.line);
    }

    /// A machine that has never pulled a model has no cache directory, and
    /// answering "unknown" there would be a regression from a usable answer.
    #[test]
    fn an_unreadable_cache_falls_back_to_the_cwd_not_to_unknown() {
        let f = disk_finding(None, Some(PLENTY));
        assert!(f.line.contains('.'), "got: {}", f.line);
        assert!(!f.problem, "plenty of space is not a fault");
    }

    /// Only when NEITHER can be read is it unknown — reported as ok, because
    /// an unreadable `df` says nothing about whether there is room.
    #[test]
    fn unknown_only_when_nothing_could_be_measured() {
        let f = disk_finding(None, None);
        assert!(!f.problem, "an unmeasurable box must not be failed");
    }
}
