// SPDX-License-Identifier: AGPL-3.0-only

//! `bench.yaml`: what this node needs to know before it may run a gate.
//!
//! Lives beside `peers.json`. Absent, the bench surface is disabled and says
//! so; present but wrong, `agent run` refuses to start naming the key —
//! there is no default for an Atlas checkout, a signing home or a CUDA
//! toolchain, and a guessed one would produce records that name the wrong
//! box or verify against nobody's key.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "bench.yaml";

/// The configuration, as written by the operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchConfig {
    /// A git checkout of Atlas. Must have `allowed_remote`.
    pub atlas_repo: PathBuf,
    /// `ATLAS_HOME` for the child: the signing identity and run history.
    pub atlas_home: PathBuf,
    /// The box class the records will name (`gb10`).
    pub hardware: String,
    /// Environment the child gets beyond the scrubbed base. `PATH_PREPEND` is
    /// prepended to `PATH`; every other key is exported as is.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Worktrees, builds, jobs and the shared cargo target directory.
    pub cache_dir: PathBuf,
    /// The remote a submitted commit must be reachable from.
    #[serde(default = "default_remote")]
    pub allowed_remote: String,
    /// Accept a commit the remote does not have. Off: a job may only run
    /// code that is already published where this node's operator trusts.
    #[serde(default)]
    pub allow_unpublished_shas: bool,
    #[serde(default = "default_queue_depth")]
    pub queue_depth: u32,
    /// Ceiling on any job's run phase, seconds.
    #[serde(default = "default_max_run_s")]
    pub max_run_s: u32,
    #[serde(default = "default_build_timeout_s")]
    pub build_timeout_s: u32,
    /// No child output for this long during the run phase → timed out.
    #[serde(default = "default_stall_timeout_s")]
    pub stall_timeout_s: u32,
    #[serde(default = "default_cancel_grace_s")]
    pub cancel_grace_s: u32,
    /// `MemAvailable / MemTotal` a job needs before it may start; the same
    /// bar Atlas's own self-start applies.
    #[serde(default = "default_min_free_fraction")]
    pub min_free_fraction: f64,
    #[serde(default = "default_min_free_disk_bytes")]
    pub min_free_disk_bytes: u64,
    /// Built binaries kept, LRU by build time. The running job's is never
    /// evicted.
    #[serde(default = "default_keep_builds")]
    pub keep_builds: u32,
    #[serde(default = "default_retain_jobs")]
    pub retain_jobs: u32,
    #[serde(default = "default_retain_days")]
    pub retain_days: u32,
    /// Run `spark sync-recipes` when the home has no recipe index.
    #[serde(default = "default_true")]
    pub sync_recipes: bool,
    /// Extra files to hand back, as globs relative to the child's home,
    /// e.g. `.atlas/artifacts/bfcl/responses-*.jsonl`.
    #[serde(default)]
    pub collect_extra: Vec<String>,
}

fn default_remote() -> String {
    "origin".into()
}
fn default_queue_depth() -> u32 {
    2
}
fn default_max_run_s() -> u32 {
    10_800
}
fn default_build_timeout_s() -> u32 {
    3600
}
fn default_stall_timeout_s() -> u32 {
    1800
}
fn default_cancel_grace_s() -> u32 {
    30
}
fn default_min_free_fraction() -> f64 {
    0.85
}
fn default_min_free_disk_bytes() -> u64 {
    20 * 1024 * 1024 * 1024
}
fn default_keep_builds() -> u32 {
    5
}
fn default_retain_jobs() -> u32 {
    50
}
fn default_retain_days() -> u32 {
    7
}
fn default_true() -> bool {
    true
}

/// Why the bench surface is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disabled(pub String);

impl BenchConfig {
    /// Load `<dir>/bench.yaml`. `Ok(Err(Disabled))` means there is no file —
    /// the agent runs without bench and reports why; `Err` means the file is
    /// there and unusable, which is fatal to `agent run`.
    ///
    /// # Errors
    /// If the file exists but does not parse or names something that is not
    /// there.
    pub fn load(dir: &Path) -> Result<Result<Self, Disabled>> {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Err(Disabled(format!(
                    "no {} — write one to enable benchmark jobs (see docs/BENCH.md)",
                    path.display()
                ))));
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let cfg: Self = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("{} does not parse", path.display()))?;
        cfg.validate()
            .with_context(|| format!("{} is unusable", path.display()))?;
        Ok(Ok(cfg))
    }

    /// Everything the file names must exist and be what it says.
    ///
    /// # Errors
    /// Naming the key.
    pub fn validate(&self) -> Result<()> {
        if !self.atlas_repo.join(".git").exists() && !self.atlas_repo.join("HEAD").exists() {
            bail!(
                "atlas_repo {} is not a git checkout",
                self.atlas_repo.display()
            );
        }
        if !self.atlas_home.is_dir() {
            bail!(
                "atlas_home {} is not a directory",
                self.atlas_home.display()
            );
        }
        if self.hardware.is_empty()
            || !self
                .hardware
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            bail!("hardware {:?} is not a box class like gb10", self.hardware);
        }
        for (k, v) in &self.env {
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                bail!("env key {k:?} is not a valid variable name");
            }
            if v.chars().any(char::is_control) {
                bail!("env {k} contains a control character");
            }
        }
        if let Some(prepend) = self.env.get("PATH_PREPEND") {
            for p in prepend.split(':') {
                if !Path::new(p).is_dir() {
                    bail!("env.PATH_PREPEND names {p}, which is not a directory");
                }
            }
        }
        if self.queue_depth == 0 {
            bail!("queue_depth must be at least 1");
        }
        if !(0.0..=1.0).contains(&self.min_free_fraction) {
            bail!(
                "min_free_fraction must be within 0..=1, got {}",
                self.min_free_fraction
            );
        }
        if self.max_run_s == 0 || self.build_timeout_s == 0 || self.stall_timeout_s == 0 {
            bail!("max_run_s, build_timeout_s and stall_timeout_s must be positive");
        }
        if self.allowed_remote.is_empty() || self.allowed_remote.contains('/') {
            bail!(
                "allowed_remote {:?} is not a remote name",
                self.allowed_remote
            );
        }
        Ok(())
    }

    pub fn jobs_dir(&self) -> PathBuf {
        self.cache_dir.join("jobs")
    }
    pub fn builds_dir(&self) -> PathBuf {
        self.cache_dir.join("build")
    }
    pub fn worktrees_dir(&self) -> PathBuf {
        self.cache_dir.join("worktrees")
    }
    pub fn target_dir(&self) -> PathBuf {
        self.cache_dir.join("target")
    }

    /// The child's environment: a scrubbed base plus what the file says.
    #[must_use]
    pub fn child_env(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for key in ["HOME", "USER", "LANG", "TERM"] {
            if let Ok(v) = std::env::var(key) {
                out.push((key.into(), v));
            }
        }
        let base_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
        let path = match self.env.get("PATH_PREPEND") {
            Some(p) => format!("{p}:{base_path}"),
            None => base_path,
        };
        out.push(("PATH".into(), path));
        out.push(("ATLAS_HOME".into(), self.atlas_home.display().to_string()));
        out.push((
            "CARGO_TARGET_DIR".into(),
            self.target_dir().display().to_string(),
        ));
        for (k, v) in &self.env {
            if k != "PATH_PREPEND" {
                out.push((k.clone(), v.clone()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "bench-config-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("repo/.git")).unwrap();
        std::fs::create_dir_all(p.join("home")).unwrap();
        p
    }

    fn minimal(p: &Path) -> String {
        format!(
            "atlas_repo: {}\natlas_home: {}\nhardware: gb10\ncache_dir: {}\n",
            p.join("repo").display(),
            p.join("home").display(),
            p.join("cache").display()
        )
    }

    #[test]
    fn no_file_means_disabled_with_a_reason_not_an_error() {
        let p = tmp();
        let r = BenchConfig::load(&p).unwrap();
        assert!(r.unwrap_err().0.contains("bench.yaml"));
    }

    #[test]
    fn a_minimal_file_loads_with_the_documented_defaults() {
        let p = tmp();
        std::fs::write(p.join(FILE), minimal(&p)).unwrap();
        let cfg = BenchConfig::load(&p).unwrap().unwrap();
        assert_eq!(cfg.allowed_remote, "origin");
        assert_eq!(cfg.queue_depth, 2);
        assert!(!cfg.allow_unpublished_shas);
        assert!((cfg.min_free_fraction - 0.85).abs() < f64::EPSILON);
        let env = cfg.child_env();
        assert!(
            env.iter()
                .any(|(k, v)| k == "ATLAS_HOME" && v.ends_with("home"))
        );
        assert!(env.iter().any(|(k, _)| k == "CARGO_TARGET_DIR"));
    }

    /// NEGATIVE CONTROLS: every named thing must exist; an unknown key is a
    /// typo, not a silent no-op.
    #[test]
    fn a_wrong_file_is_refused_naming_the_key() {
        let p = tmp();
        let cases = [
            ("atlas_repo: /nope\n", "atlas_repo"),
            ("atlas_home: /nope\n", "atlas_home"),
            ("hardware: 'gb 10'\n", "hardware"),
            ("queue_depth: 0\n", "queue_depth"),
            ("min_free_fraction: 1.5\n", "min_free_fraction"),
            ("allowed_remote: a/b\n", "allowed_remote"),
            (
                "env:\n  PATH_PREPEND: /definitely/not/here\n",
                "PATH_PREPEND",
            ),
            ("typo_key: 1\n", "typo_key"),
        ];
        for (extra, needle) in cases {
            let mut text = minimal(&p);
            // Overrides replace the minimal line of the same key.
            let key = extra.split(':').next().unwrap();
            text = text
                .lines()
                .filter(|l| !l.starts_with(key))
                .map(|l| format!("{l}\n"))
                .collect();
            text.push_str(extra);
            std::fs::write(p.join(FILE), &text).unwrap();
            let err = format!("{:#}", BenchConfig::load(&p).unwrap_err());
            assert!(err.contains(needle), "{extra:?} → {err}");
        }
    }

    #[test]
    fn path_prepend_goes_first_and_is_not_exported_itself() {
        let p = tmp();
        let text = format!(
            "{}env:\n  PATH_PREPEND: /usr\n  CUDARC_CUDA_VERSION: '13000'\n",
            minimal(&p)
        );
        std::fs::write(p.join(FILE), text).unwrap();
        let cfg = BenchConfig::load(&p).unwrap().unwrap();
        let env = cfg.child_env();
        let path = &env.iter().find(|(k, _)| k == "PATH").unwrap().1;
        assert!(path.starts_with("/usr:"), "{path}");
        assert!(!env.iter().any(|(k, _)| k == "PATH_PREPEND"));
        assert!(
            env.iter()
                .any(|(k, v)| k == "CUDARC_CUDA_VERSION" && v == "13000")
        );
    }
}
