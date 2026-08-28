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
    for f in [check_config_dir(), check_agent(), check_reachable()] {
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
    let installed = which("sparkrun").is_some();
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

/// Find a binary on PATH.
fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
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
mod tests {
    use super::*;

    #[test]
    fn which_finds_a_binary_that_exists_and_not_one_that_does_not() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }
}
