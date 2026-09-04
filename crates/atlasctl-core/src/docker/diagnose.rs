// SPDX-License-Identifier: AGPL-3.0-only

//! Telling apart the ways `docker` refuses.
//!
//! atlasctl never speaks to the daemon socket directly — it shells out to the
//! `docker` CLI and reads what comes back. That leaves exactly one signal for
//! "why did this not work": the CLI's own stderr. Passing that through verbatim
//! is what produced the report this module exists to answer, where a user on a
//! working DGX Spark was shown
//!
//! ```text
//! permission denied while trying to connect to the Docker daemon socket
//! at unix:///var/run/docker.sock
//! ```
//!
//! and left to work out on their own that the fix is a one-time group change.
//! Worse, the agent's own probe called that case "the docker daemon did not
//! answer", which is the opposite of what happened: the daemon answered and
//! refused. That wrong diagnosis then travelled to the browser and was rendered
//! as a hardware verdict about the machine.
//!
//! This is deliberately a pure function over `(status, stderr)`. Classifying is
//! the part that is easy to get wrong and easy to test; running the process is
//! neither.

use std::fmt;

/// The canonical place to send someone whose Docker cannot be reached.
///
/// Ours rather than Docker's, so the page can carry the DGX Spark specifics and
/// the "do not use sudo" warning, and link onward for the canonical steps.
pub const DOCKER_SETUP_DOCS: &str =
    "https://docs.atlasinference.io/getting-started/troubleshooting.html";

/// Why a `docker` invocation did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerFault {
    /// No `docker` on PATH at all.
    NotInstalled,
    /// The daemon answered and refused this user. Almost always group membership.
    PermissionDenied,
    /// Nothing is listening: the daemon is not running, or DOCKER_HOST is wrong.
    DaemonDown,
    /// Something else. The raw stderr is carried so it is never swallowed.
    Other(String),
}

impl DockerFault {
    /// Classify a failed `docker` invocation from its exit status and stderr.
    ///
    /// Order matters, though not for the reason it first appears to. Docker's
    /// usual group-denial message says "trying to connect to the Docker daemon
    /// socket", which the daemon-down patterns below do NOT match, so for that
    /// one string either order works.
    ///
    /// It matters for the clients that print both facts. Several — older
    /// engines, Docker Desktop's shim, some podman-docker wrappers — emit the
    /// "Cannot connect to the Docker daemon ... Is the docker daemon running?"
    /// line and a "permission denied" cause together. Classified daemon-first,
    /// those users are told to start a daemon that is already running. The
    /// permission check therefore comes first, and `both_phrases_present_...`
    /// below pins it with exactly such a string.
    pub fn classify(stderr: &str) -> Self {
        let s = stderr.to_ascii_lowercase();

        if s.contains("permission denied") || s.contains("permissiondenied") {
            return Self::PermissionDenied;
        }
        // The shell's phrasing and the OS's, since atlasctl may be invoked
        // where `docker` is genuinely absent rather than merely unreachable.
        if s.contains("command not found")
            || s.contains("no such file or directory")
            || s.contains("executable file not found")
            || s.contains("is not recognized as an internal or external command")
        {
            return Self::NotInstalled;
        }
        if s.contains("cannot connect to the docker daemon")
            || s.contains("is the docker daemon running")
            || s.contains("connection refused")
            || s.contains("docker_host")
        {
            return Self::DaemonDown;
        }
        Self::Other(stderr.trim().to_owned())
    }

    /// A one-line statement of what is wrong. No remedy, no URL.
    ///
    /// This is what travels over the wire to the browser as the launchability
    /// reason, so it has to read well inside a sentence someone else wrote.
    pub fn summary(&self) -> String {
        match self {
            Self::NotInstalled => "Docker is not installed, or not on PATH".to_owned(),
            Self::PermissionDenied => {
                "Docker refused this user: you are not in the `docker` group".to_owned()
            }
            Self::DaemonDown => "the Docker daemon is not running".to_owned(),
            Self::Other(raw) => format!("Docker failed: {raw}"),
        }
    }

    /// The full terminal message: what is wrong, how to fix it, where to read more.
    ///
    /// The `sudo` warning is not decoration. The reported user's next move after
    /// the permission error was `sudo atlasctl`, which appears to work and then
    /// runs the model as root and leaves root-owned files in `~/.atlas` that the
    /// unprivileged run afterwards cannot read.
    pub fn advice(&self) -> String {
        match self {
            Self::PermissionDenied => format!(
                "Docker is installed but this user cannot reach it.\n\
                 \n\
                 You are not in the `docker` group. Fix it once:\n\
                 \n    sudo usermod -aG docker $USER\
                 \n    newgrp docker          # or log out and back in\n\
                 \n\
                 Then re-run. Do NOT use `sudo atlasctl` — it runs the model as\n\
                 root and leaves root-owned files in ~/.atlas that your normal\n\
                 user cannot read afterwards.\n\
                 \n\
                 {DOCKER_SETUP_DOCS}"
            ),
            Self::NotInstalled => format!(
                "Docker is not installed, or not on PATH.\n\
                 \n\
                 `atlasctl run` needs a container engine. `atlasctl list` and\n\
                 `atlasctl run --print` work without one.\n\
                 \n\
                 {DOCKER_SETUP_DOCS}"
            ),
            Self::DaemonDown => format!(
                "Docker is installed but its daemon is not running.\n\
                 \n\
                 Start it, then re-run:\n\
                 \n    sudo systemctl start docker\n\
                 \n\
                 {DOCKER_SETUP_DOCS}"
            ),
            Self::Other(raw) => format!(
                "Docker failed and atlasctl does not recognise the error:\n\
                 \n{raw}\n\
                 \n{DOCKER_SETUP_DOCS}"
            ),
        }
    }
}

impl fmt::Display for DockerFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact string Docker emits for the group case, from the user report.
    const REAL_PERMISSION_DENIED: &str = "Got permission denied while trying to connect to the \
         Docker daemon socket at unix:///var/run/docker.sock: Get \
         \"http://%2Fvar%2Frun%2Fdocker.sock/v1.47/info\": dial unix \
         /var/run/docker.sock: connect: permission denied";

    /// The exact string Docker emits when nothing is listening.
    const REAL_DAEMON_DOWN: &str = "Cannot connect to the Docker daemon at \
         unix:///var/run/docker.sock. Is the docker daemon running?";

    #[test]
    fn the_reported_permission_error_is_classified_as_a_permission_problem() {
        assert_eq!(
            DockerFault::classify(REAL_PERMISSION_DENIED),
            DockerFault::PermissionDenied
        );
    }

    #[test]
    fn both_phrases_present_still_means_permission_not_a_dead_daemon() {
        // This is the case the check ORDER exists for, and the reason the
        // ordering is testable at all: a client that reports both facts in one
        // stderr. Classified daemon-first, the operator is told to start a
        // daemon that is already running.
        //
        // Written as a test after the first attempt at this file asserted the
        // ordering against REAL_PERMISSION_DENIED — which does not contain the
        // daemon-down phrasing, so reversing the checks left it passing and the
        // "control" proved nothing.
        let both = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                    Is the docker daemon running?: dial unix /var/run/docker.sock: \
                    connect: permission denied";
        assert_eq!(DockerFault::classify(both), DockerFault::PermissionDenied);
    }

    #[test]
    fn a_genuinely_dead_daemon_is_still_reported_as_one() {
        assert_eq!(
            DockerFault::classify(REAL_DAEMON_DOWN),
            DockerFault::DaemonDown
        );
    }

    #[test]
    fn a_missing_binary_is_told_apart_from_an_unreachable_one() {
        for s in [
            "docker: command not found",
            "exec: \"docker\": executable file not found in $PATH",
            "'docker' is not recognized as an internal or external command",
        ] {
            assert_eq!(DockerFault::classify(s), DockerFault::NotInstalled, "{s}");
        }
    }

    #[test]
    fn an_unrecognised_error_keeps_its_text_rather_than_being_swallowed() {
        let f = DockerFault::classify("  toomanyrequests: rate limit exceeded  ");
        assert_eq!(
            f,
            DockerFault::Other("toomanyrequests: rate limit exceeded".to_owned())
        );
        // And it still reaches the operator.
        assert!(f.advice().contains("rate limit exceeded"));
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(
            DockerFault::classify("PERMISSION DENIED"),
            DockerFault::PermissionDenied
        );
    }

    #[test]
    fn the_permission_advice_carries_the_fix_the_warning_and_the_link() {
        let a = DockerFault::PermissionDenied.advice();
        assert!(
            a.contains("usermod -aG docker"),
            "must name the one-time fix"
        );
        assert!(a.contains("newgrp docker"), "must say how to apply it now");
        assert!(
            a.contains("sudo atlasctl"),
            "must warn against the obvious wrong move"
        );
        assert!(a.contains(DOCKER_SETUP_DOCS), "must link the docs");
    }

    #[test]
    fn no_summary_claims_the_daemon_was_silent_when_it_answered() {
        // The original defect, as an assertion: the permission case must never
        // be described as the daemon failing to answer.
        let s = DockerFault::PermissionDenied.summary().to_ascii_lowercase();
        assert!(!s.contains("did not answer"));
        assert!(!s.contains("not running"));
        assert!(s.contains("group"));
    }

    #[test]
    fn every_variant_has_a_nonempty_summary_and_advice() {
        // Guards against a new variant landing with an empty arm.
        for f in [
            DockerFault::NotInstalled,
            DockerFault::PermissionDenied,
            DockerFault::DaemonDown,
            DockerFault::Other("x".into()),
        ] {
            assert!(!f.summary().is_empty(), "{f:?}");
            assert!(
                f.advice().len() > f.summary().len(),
                "{f:?} advice adds nothing"
            );
        }
    }
}
