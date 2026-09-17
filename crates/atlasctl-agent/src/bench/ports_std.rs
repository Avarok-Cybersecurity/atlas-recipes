// SPDX-License-Identifier: AGPL-3.0-only

//! The real [`Ports`]: git, cargo, a child in its own process group, and
//! `/proc`. Linux only — the boxes that run gates are.
//!
//! The build cache is `<cache>/build/<sha>/{spark,provenance.json}`. A hit
//! requires all three: provenance parses, names this sha, and the binary on
//! disk hashes to what provenance recorded. `spark` embeds no git sha, so
//! provenance is the only witness, and a binary that does not hash to its
//! provenance is a binary somebody replaced.

use super::child::{
    kill_group, parse_progress, parse_verdict, same_process, sanitize, start_ticks,
};
use super::config::BenchConfig;
use super::job::write_atomic;
use super::machine::{
    BuildResult, CacheMiss, CachedBinary, ChildHandle, Ports, RecordState, RunEnd, RunPlan,
};
use super::records;
use anyhow::{Context, Result, bail};
use atlasctl_protocol::msg::bench::{MAX_LOG_LINES_PER_EVENT, Sha};
use atlasctl_protocol::msg::bench_event::{ArtifactMeta, EventKind, LogStream, Verdict};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// What a build leaves beside its binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub sha: Sha,
    pub binary_sha256: String,
    pub built_at_s: u64,
    pub bytes: u64,
}

pub struct StdPorts {
    pub cfg: BenchConfig,
    pub cancel_grace: Duration,
    /// Children this process spawned, by pid, so their exit codes can be
    /// collected. A child recovered after a restart has no handle here and
    /// is watched through `/proc` alone (its verdict line still tells the
    /// truth; only the raw exit code reads -1).
    pub children: std::sync::Mutex<std::collections::HashMap<u32, std::process::Child>>,
}

impl StdPorts {
    pub fn new(cfg: BenchConfig) -> Self {
        let cancel_grace = Duration::from_secs(u64::from(cfg.cancel_grace_s));
        Self {
            cfg,
            cancel_grace,
            children: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl StdPorts {
    fn git(&self, args: &[&str]) -> Result<std::process::Output> {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&self.cfg.atlas_repo)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .with_context(|| format!("running git {args:?}"))
    }

    fn git_ok(&self, args: &[&str]) -> Result<String> {
        let out = self.git(args)?;
        if !out.status.success() {
            bail!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn build_dir(&self, sha: &Sha) -> PathBuf {
        self.cfg.builds_dir().join(sha.as_str())
    }

    fn spark_cmd(&self, binary: &Path, args: &[&str]) -> Result<String> {
        let mut c = std::process::Command::new(binary);
        c.args(args)
            .stdin(std::process::Stdio::null())
            .env_clear()
            .envs(self.cfg.child_env());
        let out = c
            .output()
            .with_context(|| format!("running {} {args:?}", binary.display()))?;
        if !out.status.success() {
            bail!(
                "{} {}: {}",
                binary.display(),
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

pub fn sha256_file(path: &Path) -> Result<(String, u64)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n: u64 = 0;
    loop {
        let read = f.read(&mut buf)?;
        if read == 0 {
            break;
        }
        h.update(&buf[..read]);
        n += read as u64;
    }
    Ok((hex::encode(h.finalize()), n))
}

impl Ports for StdPorts {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    fn has_commit(&self, sha: &Sha) -> Result<bool> {
        Ok(self
            .git(&["cat-file", "-e", &format!("{sha}^{{commit}}")])?
            .status
            .success())
    }

    fn fetch(&self) -> Result<()> {
        self.git_ok(&["fetch", "--quiet", "--", &self.cfg.allowed_remote])
            .map(|_| ())
    }

    fn published(&self, sha: &Sha) -> Result<bool> {
        let out = self.git_ok(&["branch", "-r", "--contains", sha.as_str()])?;
        let prefix = format!("{}/", self.cfg.allowed_remote);
        Ok(out.lines().any(|l| l.trim().starts_with(&prefix)))
    }

    fn worktree(&self, sha: &Sha) -> Result<PathBuf> {
        let dir = self.cfg.worktrees_dir().join(sha.as_str());
        if dir.join(".git").exists() {
            let head = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["rev-parse", "HEAD"])
                .output()?;
            if String::from_utf8_lossy(&head.stdout).trim() == sha.as_str() {
                return Ok(dir);
            }
            let _ = self.git(&["worktree", "remove", "--force", &dir.display().to_string()]);
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(self.cfg.worktrees_dir())?;
        self.git_ok(&[
            "worktree",
            "add",
            "--detach",
            "--",
            &dir.display().to_string(),
            sha.as_str(),
        ])?;
        Ok(dir)
    }

    fn cached_binary(&self, sha: &Sha) -> Result<Result<CachedBinary, CacheMiss>> {
        let dir = self.build_dir(sha);
        let bin = dir.join("spark");
        if !bin.exists() {
            return Ok(Err(CacheMiss::NoBuild));
        }
        let prov_path = dir.join("provenance.json");
        let Ok(text) = std::fs::read_to_string(&prov_path) else {
            return Ok(Err(CacheMiss::NoProvenance));
        };
        let Ok(prov) = serde_json::from_str::<Provenance>(&text) else {
            return Ok(Err(CacheMiss::NoProvenance));
        };
        if prov.sha != *sha {
            return Ok(Err(CacheMiss::WrongSha(prov.sha.short().to_string())));
        }
        let (actual, _) = sha256_file(&bin)?;
        if actual != prov.binary_sha256 {
            return Ok(Err(CacheMiss::HashMismatch));
        }
        Ok(Ok(CachedBinary {
            path: bin,
            sha256: actual,
        }))
    }

    fn build(&self, sha: &Sha, worktree: &Path, cancel: &AtomicBool) -> Result<BuildResult> {
        let started = Instant::now();
        std::fs::create_dir_all(self.cfg.target_dir())?;
        let log = self.build_dir(sha).join("build.log");
        std::fs::create_dir_all(self.build_dir(sha))?;
        let logf = std::fs::File::create(&log)?;
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["build", "--release", "--bin", "spark"])
            .current_dir(worktree)
            .env_clear()
            .envs(self.cfg.child_env())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(logf.try_clone()?))
            .stderr(std::process::Stdio::from(logf));
        let mut child = super::child::in_own_process_group(&mut cmd)?
            .spawn()
            .context("spawning cargo")?;
        let timeout = Duration::from_secs(u64::from(self.cfg.build_timeout_s));
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    bail!("cargo build failed (see {})", log.display());
                }
                break;
            }
            if cancel.load(Ordering::SeqCst) {
                kill_group(child.id(), "-TERM");
                let _ = child.wait();
                bail!("build cancelled");
            }
            if started.elapsed() > timeout {
                kill_group(child.id(), "-KILL");
                let _ = child.wait();
                bail!("build exceeded {timeout:?}");
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let built = self.cfg.target_dir().join("release").join("spark");
        let dest = self.build_dir(sha).join("spark");
        std::fs::copy(&built, &dest)
            .with_context(|| format!("copying {} into the cache", built.display()))?;
        let (sha256, bytes) = sha256_file(&dest)?;
        let prov = Provenance {
            sha: sha.clone(),
            binary_sha256: sha256.clone(),
            built_at_s: self.now_ms() / 1000,
            bytes,
        };
        write_atomic(
            &self.build_dir(sha).join("provenance.json"),
            &serde_json::to_vec_pretty(&prov)?,
        )?;
        Ok(BuildResult {
            binary: CachedBinary { path: dest, sha256 },
            secs: started.elapsed().as_secs(),
        })
    }

    fn ensure_recipes(&self, binary: &Path) -> Result<bool> {
        if !self.cfg.sync_recipes
            || self
                .cfg
                .atlas_home
                .join("atlas-recipes")
                .join("index.json")
                .exists()
        {
            return Ok(false);
        }
        self.spark_cmd(binary, &["sync-recipes"])?;
        Ok(true)
    }

    fn known_gates(&self, binary: &Path) -> Result<Vec<String>> {
        let text = self.spark_cmd(binary, &["benchmark", "list", "--format", "json"])?;
        let v: serde_json::Value =
            serde_json::from_str(&text).context("benchmark list is not JSON")?;
        let ids = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d.get("id").and_then(|i| i.as_str()).map(str::to_owned))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            bail!("benchmark list named no benchmarks");
        }
        Ok(ids)
    }

    fn bad_params(&self, binary: &Path, gate: &str, params: &[String]) -> Result<Vec<String>> {
        if params.is_empty() {
            return Ok(vec![]);
        }
        let text = self.spark_cmd(binary, &["benchmark", "list", gate, "--format", "json"])?;
        let v: serde_json::Value =
            serde_json::from_str(&text).context("benchmark schema is not JSON")?;
        let known: Vec<String> = v
            .get("params")
            .or_else(|| v.get("parameters"))
            .and_then(|p| p.as_array())
            .into_iter()
            .flatten()
            .filter_map(|p| {
                p.get("key")
                    .or_else(|| p.get("name"))
                    .and_then(|k| k.as_str())
                    .map(str::to_owned)
            })
            .collect();
        Ok(params
            .iter()
            .filter(|p| !known.contains(p))
            .cloned()
            .collect())
    }

    fn spawn(&self, plan: &RunPlan) -> Result<ChildHandle> {
        let (program, args) = plan.argv.split_first().context("empty argv")?;
        let logf = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&plan.log_path)?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .current_dir(&plan.cwd)
            .env_clear()
            .envs(plan.env.iter().cloned())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(logf.try_clone()?))
            .stderr(std::process::Stdio::from(logf));
        let child = super::child::in_own_process_group(&mut cmd)?
            .spawn()
            .with_context(|| format!("spawning {program}"))?;
        let pid = child.id();
        let ticks = start_ticks(pid).unwrap_or(0);
        self.children
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(pid, child);
        Ok(ChildHandle {
            pid,
            start_ticks: ticks,
        })
    }

    fn wait(
        &self,
        child: &ChildHandle,
        plan: &RunPlan,
        cancel: &AtomicBool,
        emit: &dyn Fn(EventKind),
    ) -> Result<RunEnd> {
        use std::io::{BufRead, Seek};
        let started = Instant::now();
        let mut last_output = Instant::now();
        let mut offset: u64 = 0;
        let mut verdict: Option<Verdict> = None;
        let mut pending: Vec<String> = Vec::new();
        let mut killing: Option<(RunEnd, Instant)> = None;
        loop {
            // Tail the log file from where we left off.
            if let Ok(mut f) = std::fs::File::open(&plan.log_path) {
                let _ = f.seek(std::io::SeekFrom::Start(offset));
                let mut reader = std::io::BufReader::new(f);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line) {
                    if n == 0 || !line.ends_with('\n') {
                        break;
                    }
                    offset += n as u64;
                    last_output = Instant::now();
                    let clean = sanitize(line.trim_end_matches('\n'));
                    if let Some(v) = parse_verdict(&clean) {
                        verdict = Some(v);
                    } else if let Some(p) = parse_progress(&clean) {
                        emit(p);
                    }
                    pending.push(clean);
                    if pending.len() >= MAX_LOG_LINES_PER_EVENT {
                        emit(EventKind::Log {
                            stream: LogStream::Run,
                            lines: std::mem::take(&mut pending),
                        });
                    }
                    line.clear();
                }
            }
            if !pending.is_empty() {
                emit(EventKind::Log {
                    stream: LogStream::Run,
                    lines: std::mem::take(&mut pending),
                });
            }
            // Our own child: collect its status. A recovered one: watch /proc.
            let exited: Option<i32> = {
                let mut kids = self.children.lock().unwrap_or_else(|p| p.into_inner());
                match kids.get_mut(&child.pid) {
                    Some(c) => match c.try_wait()? {
                        Some(status) => {
                            kids.remove(&child.pid);
                            Some(status.code().unwrap_or(-1))
                        }
                        None => None,
                    },
                    None => (!same_process(child.pid, child.start_ticks)).then_some(-1),
                }
            };
            if let Some(code) = exited {
                if let Some((end, _)) = killing {
                    return Ok(end);
                }
                return Ok(RunEnd::Exited { code, verdict });
            }
            if let Some((_, since)) = &killing {
                if since.elapsed() > self.cancel_grace {
                    kill_group(child.pid, "-KILL");
                }
            } else if cancel.load(Ordering::SeqCst) {
                kill_group(child.pid, "-TERM");
                killing = Some((RunEnd::Cancelled, Instant::now()));
            } else if started.elapsed() > plan.max_run {
                kill_group(child.pid, "-TERM");
                killing = Some((
                    RunEnd::TimedOut {
                        after_s: started.elapsed().as_secs(),
                    },
                    Instant::now(),
                ));
            } else if last_output.elapsed() > plan.stall_timeout {
                kill_group(child.pid, "-TERM");
                killing = Some((
                    RunEnd::Stalled {
                        after_s: last_output.elapsed().as_secs(),
                    },
                    Instant::now(),
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn record_state(&self, worktree: &Path, gate: &str) -> Result<RecordState> {
        Ok(records::state(&worktree.join(".benchmarks").join(gate)))
    }

    fn collect(
        &self,
        worktree: &Path,
        gate: &str,
        before: &RecordState,
        dest: &Path,
    ) -> Result<Vec<ArtifactMeta>> {
        records::collect(worktree, gate, before, dest)
    }
}

#[cfg(test)]
#[path = "ports_std_tests.rs"]
mod ports_std_tests;
