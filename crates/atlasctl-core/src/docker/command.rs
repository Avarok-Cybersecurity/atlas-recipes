// SPDX-License-Identifier: AGPL-3.0-only

//! The structured `docker run` invocation.

use super::quote::{shell_join, shell_quote};
use std::collections::BTreeMap;
use std::fmt;

/// The user a container runs as, resolved to concrete ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserSpec {
    /// Host uid.
    pub uid: u32,
    /// Host gid.
    pub gid: u32,
}

/// A fully-resolved `docker run`, as structure rather than text.
///
/// Keeping this a value rather than a string is what lets one launch produce
/// three faithful renderings: an argv that is executed directly (never through
/// a shell), a shell-quoted line for the copy button, and a portable line that
/// keeps `$(id -u)` and `$HOME` unexpanded so it can be pasted on another box.
/// All three come from this one value, so they cannot drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerCommand {
    /// Run detached (`-d`).
    pub detach: bool,
    /// `--entrypoint`; `Some("")` clears the image's own.
    pub entrypoint: Option<String>,
    /// `--privileged`.
    pub privileged: bool,
    /// Accelerator flags, supplied by the vendor's device profile.
    pub device_flags: Vec<String>,
    /// `--ipc=`.
    pub ipc: String,
    /// `--shm-size=`.
    pub shm_size: String,
    /// `--network=`.
    pub network: String,
    /// `--user`, plus the passwd/group mounts that make it usable.
    pub user: Option<UserSpec>,
    /// `--security-opt` values.
    pub security_opts: Vec<String>,
    /// `--cap-add` values.
    pub cap_add: Vec<String>,
    /// `--ulimit` values.
    pub ulimits: Vec<String>,
    /// `--device` values.
    pub devices: Vec<String>,
    /// `--memory=`, when bounded.
    pub memory: Option<String>,
    /// `--label` pairs, so containers can be rediscovered after a restart.
    pub labels: Vec<(String, String)>,
    /// `--rm`.
    pub auto_remove: bool,
    /// `--restart`.
    pub restart: Option<String>,
    /// `--name`.
    pub name: String,
    /// Environment, sorted by key so output is stable.
    pub env: BTreeMap<String, String>,
    /// Volume mounts, sorted by host path so output is stable.
    pub volumes: BTreeMap<String, String>,
    /// The image to run.
    pub image: String,
    /// The command argv inside the container.
    pub command: Vec<String>,
}

/// One rendered argument, and whether the shell is meant to interpret it.
///
/// The flag is set where the renderer WRITES a substitution, never inferred
/// from the text afterwards. That distinction is the whole security property:
/// `is_symbolic` used to decide by substring, so any argument that merely
/// CONTAINED `$(id -u)` was emitted unquoted — including a recipe's own `env:`
/// value, which reaches argv as `-e KEY=<value>` and comes from a remote
/// index. A recipe could therefore put `$(curl … | sh)` beside `$(id -u)` in
/// one value and have it printed raw into the line an operator is told to
/// paste. The old doc comment stated the correct rule — "any other `$` came
/// from recipe data and must still be quoted" — and the substring test did not
/// implement it.
#[derive(Debug, Clone)]
struct Arg {
    text: String,
    /// True only for text this renderer produced itself.
    symbolic: bool,
}

impl From<String> for Arg {
    fn from(text: String) -> Self {
        Self {
            text,
            symbolic: false,
        }
    }
}

impl From<&str> for Arg {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            symbolic: false,
        }
    }
}

impl Arg {
    /// An argument the renderer wrote a substitution into on purpose.
    fn symbolic(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            symbolic: true,
        }
    }
}

/// Which rendering of the user block to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserRender {
    /// Concrete ids, for execution.
    Resolved,
    /// `$(id -u):$(id -g)`, for a command a human will paste elsewhere.
    Portable,
}

impl DockerCommand {
    /// The argv, executed directly — there is no shell anywhere in this path.
    pub fn to_argv(&self) -> Vec<String> {
        // Execution takes the text only: there is no shell on this path, so
        // the symbolic marking is meaningless here by construction.
        self.render(UserRender::Resolved, None)
            .into_iter()
            .map(|a| a.text)
            .collect()
    }

    /// A pasteable line that keeps host-specific values symbolic.
    ///
    /// `home` replaces the literal home directory in volume sources with
    /// `$HOME`, so the command the website prints is not tied to one account.
    ///
    /// Arguments carrying a deliberate shell substitution are emitted
    /// **unquoted** — quoting `$(id -u)` would turn the whole point of this
    /// rendering into a literal string, and the pasted command would fail with
    /// an unusable uid.
    pub fn display_portable(&self, home: Option<&str>) -> String {
        self.render(UserRender::Portable, home)
            .into_iter()
            .map(|a| {
                if a.symbolic {
                    a.text
                } else {
                    shell_quote(&a.text)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn render(&self, user_render: UserRender, home: Option<&str>) -> Vec<Arg> {
        let mut v: Vec<Arg> = vec!["docker".into(), "run".into()];
        if self.detach {
            v.push("-d".into());
        }
        if let Some(ep) = &self.entrypoint {
            v.push("--entrypoint".into());
            v.push(ep.clone().into());
        }
        if self.privileged {
            v.push("--privileged".into());
        }
        v.extend(self.device_flags.iter().map(|f| Arg::from(f.clone())));
        v.push(format!("--ipc={}", self.ipc).into());
        v.push(format!("--shm-size={}", self.shm_size).into());
        v.push(format!("--network={}", self.network).into());

        if let Some(u) = &self.user {
            v.push("--user".into());
            v.push(match user_render {
                UserRender::Resolved => Arg::from(format!("{}:{}", u.uid, u.gid)),
                // The one place this substitution is written, so the one place
                // it is marked. Quoting it would make the pasted command run
                // as a user literally named `$(id -u)`.
                UserRender::Portable => Arg::symbolic("$(id -u):$(id -g)"),
            });
            // Without these the container has a uid it cannot name, and tools
            // that look the user up fail in confusing ways.
            v.push("-v".into());
            v.push("/etc/passwd:/etc/passwd:ro".into());
            v.push("-v".into());
            v.push("/etc/group:/etc/group:ro".into());
        }

        for o in &self.security_opts {
            v.push("--security-opt".into());
            v.push(o.clone().into());
        }
        for c in &self.cap_add {
            v.push(format!("--cap-add={c}").into());
        }
        for u in &self.ulimits {
            v.push("--ulimit".into());
            v.push(u.clone().into());
        }
        for d in &self.devices {
            v.push("--device".into());
            v.push(d.clone().into());
        }
        if let Some(m) = &self.memory {
            v.push(format!("--memory={m}").into());
        }
        for (k, val) in &self.labels {
            v.push("--label".into());
            v.push(format!("{k}={val}").into());
        }
        if self.auto_remove {
            v.push("--rm".into());
        }
        if let Some(r) = &self.restart {
            v.push("--restart".into());
            v.push(r.clone().into());
        }
        v.push("--name".into());
        v.push(self.name.clone().into());

        for (k, val) in &self.env {
            v.push("-e".into());
            v.push(format!("{k}={val}").into());
        }
        for (host, ctr) in &self.volumes {
            // `rewritten` records whether WE substituted `$HOME`, rather than
            // asking afterwards whether the text looks like we did — a recipe
            // may name a volume that already starts with `$HOME/`, and that is
            // data, not something to let the shell expand.
            let (host, rewritten) = match (user_render, home) {
                (UserRender::Portable, Some(h)) if !h.is_empty() && host.starts_with(h) => (
                    format!(
                        "{}{}",
                        crate::platform::home_placeholder(),
                        &host[h.len()..]
                    ),
                    true,
                ),
                _ => (host.clone(), false),
            };
            v.push("-v".into());
            let spec = format!("{host}:{ctr}");
            v.push(if rewritten {
                Arg::symbolic(spec)
            } else {
                Arg::from(spec)
            });
        }

        v.push(self.image.clone().into());
        v.extend(self.command.iter().map(|c| Arg::from(c.clone())));
        v
    }
}

impl fmt::Display for DockerCommand {
    /// The shell-quoted line, byte-identical in meaning to [`Self::to_argv`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", shell_join(self.to_argv()))
    }
}

#[cfg(test)]
mod tests;
