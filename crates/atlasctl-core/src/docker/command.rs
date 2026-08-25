// SPDX-License-Identifier: AGPL-3.0-only

//! The structured `docker run` invocation.

use super::quote::shell_join;
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
        self.render(UserRender::Resolved, None)
    }

    /// A pasteable line that keeps host-specific values symbolic.
    ///
    /// `home` replaces the literal home directory in volume sources with
    /// `$HOME`, so the command the website prints is not tied to one account.
    pub fn display_portable(&self, home: Option<&str>) -> String {
        shell_join(self.render(UserRender::Portable, home))
    }

    fn render(&self, user_render: UserRender, home: Option<&str>) -> Vec<String> {
        let mut v = vec!["docker".to_string(), "run".to_string()];
        if self.detach {
            v.push("-d".into());
        }
        if let Some(ep) = &self.entrypoint {
            v.push("--entrypoint".into());
            v.push(ep.clone());
        }
        if self.privileged {
            v.push("--privileged".into());
        }
        v.extend(self.device_flags.iter().cloned());
        v.push(format!("--ipc={}", self.ipc));
        v.push(format!("--shm-size={}", self.shm_size));
        v.push(format!("--network={}", self.network));

        if let Some(u) = &self.user {
            v.push("--user".into());
            v.push(match user_render {
                UserRender::Resolved => format!("{}:{}", u.uid, u.gid),
                UserRender::Portable => "$(id -u):$(id -g)".into(),
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
            v.push(o.clone());
        }
        for c in &self.cap_add {
            v.push(format!("--cap-add={c}"));
        }
        for u in &self.ulimits {
            v.push("--ulimit".into());
            v.push(u.clone());
        }
        for d in &self.devices {
            v.push("--device".into());
            v.push(d.clone());
        }
        if let Some(m) = &self.memory {
            v.push(format!("--memory={m}"));
        }
        for (k, val) in &self.labels {
            v.push("--label".into());
            v.push(format!("{k}={val}"));
        }
        if self.auto_remove {
            v.push("--rm".into());
        }
        if let Some(r) = &self.restart {
            v.push("--restart".into());
            v.push(r.clone());
        }
        v.push("--name".into());
        v.push(self.name.clone());

        for (k, val) in &self.env {
            v.push("-e".into());
            v.push(format!("{k}={val}"));
        }
        for (host, ctr) in &self.volumes {
            let host = match (user_render, home) {
                (UserRender::Portable, Some(h)) if !h.is_empty() && host.starts_with(h) => {
                    format!("$HOME{}", &host[h.len()..])
                }
                _ => host.clone(),
            };
            v.push("-v".into());
            v.push(format!("{host}:{ctr}"));
        }

        v.push(self.image.clone());
        v.extend(self.command.iter().cloned());
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
