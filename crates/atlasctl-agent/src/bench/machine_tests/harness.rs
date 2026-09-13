// SPDX-License-Identifier: AGPL-3.0-only

//! The scripted world the machine tests drive: every port answers what its
//! field says, and the log records what was asked.

use crate::bench::job::{JobRecord, JobSpec, JobStore};
use crate::bench::journal::Journal;
use crate::bench::machine::*;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::{GateId, JobKey, Sha};
use atlasctl_protocol::msg::bench_event::{
    ArtifactKind, ArtifactMeta, EventKind, Verdict, VerdictKind,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A scripted world. Every `Result` field is what the corresponding port
/// answers; the log records what was asked.
pub(super) struct Script {
    pub(super) has_commit: Vec<bool>,
    pub(super) fetch_ok: bool,
    pub(super) published: bool,
    pub(super) cache: Result<CachedBinary, CacheMiss>,
    pub(super) build_ok: bool,
    pub(super) known: Vec<String>,
    pub(super) bad_params: Vec<String>,
    pub(super) end: RunEnd,
    pub(super) artifacts: Vec<ArtifactMeta>,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) cancel_during_build: Option<Arc<AtomicBool>>,
    pub(super) clock: Mutex<u64>,
}

impl Script {
    pub(super) fn happy() -> Self {
        Self {
            has_commit: vec![true],
            fetch_ok: true,
            published: true,
            cache: Err(CacheMiss::NoBuild),
            build_ok: true,
            known: vec!["decode-floor".into(), "bfcl-subset-a".into()],
            bad_params: vec![],
            end: RunEnd::Exited {
                code: 0,
                verdict: Some(Verdict {
                    kind: VerdictKind::Pass,
                    text: "median 26.0".into(),
                }),
            },
            artifacts: vec![
                ArtifactMeta {
                    name: "r.json".into(),
                    relative_path: ".benchmarks/decode-floor/r.json".into(),
                    bytes: 10,
                    sha256: "aa".repeat(32),
                    kind: ArtifactKind::Record,
                },
                ArtifactMeta {
                    name: "r.json.sig".into(),
                    relative_path: ".benchmarks/decode-floor/r.json.sig".into(),
                    bytes: 3,
                    sha256: "bb".repeat(32),
                    kind: ArtifactKind::Signature,
                },
            ],
            calls: Mutex::new(vec![]),
            cancel_during_build: None,
            clock: Mutex::new(1_000_000),
        }
    }
    fn log(&self, s: &str) {
        self.calls.lock().unwrap().push(s.into());
    }
    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub(super) fn binary() -> CachedBinary {
        CachedBinary {
            path: PathBuf::from("/cache/build/x/spark"),
            sha256: "cc".repeat(32),
        }
    }
}

impl Ports for Script {
    fn now_ms(&self) -> u64 {
        let mut c = self.clock.lock().unwrap();
        *c += 1000;
        *c
    }
    fn has_commit(&self, _: &Sha) -> Result<bool> {
        let n = self.calls().iter().filter(|c| *c == "has_commit").count();
        self.log("has_commit");
        Ok(*self.has_commit.get(n).unwrap_or(&true))
    }
    fn fetch(&self) -> Result<()> {
        self.log("fetch");
        if self.fetch_ok {
            Ok(())
        } else {
            anyhow::bail!("remote unreachable")
        }
    }
    fn published(&self, _: &Sha) -> Result<bool> {
        self.log("published");
        Ok(self.published)
    }
    fn worktree(&self, _: &Sha) -> Result<PathBuf> {
        self.log("worktree");
        Ok(PathBuf::from("/cache/worktrees/x"))
    }
    fn cached_binary(&self, _: &Sha) -> Result<Result<CachedBinary, CacheMiss>> {
        self.log("cached_binary");
        Ok(self.cache.clone())
    }
    fn build(&self, _: &Sha, _: &Path, cancel: &AtomicBool) -> Result<BuildResult> {
        self.log("build");
        if let Some(c) = &self.cancel_during_build {
            c.store(true, Ordering::SeqCst);
            let _ = cancel;
            anyhow::bail!("killed");
        }
        if self.build_ok {
            Ok(BuildResult {
                binary: Self::binary(),
                secs: 42,
            })
        } else {
            anyhow::bail!("cargo failed")
        }
    }
    fn ensure_recipes(&self, _: &Path) -> Result<bool> {
        self.log("ensure_recipes");
        Ok(false)
    }
    fn known_gates(&self, _: &Path) -> Result<Vec<String>> {
        self.log("known_gates");
        Ok(self.known.clone())
    }
    fn bad_params(&self, _: &Path, _: &str, _: &[String]) -> Result<Vec<String>> {
        self.log("bad_params");
        Ok(self.bad_params.clone())
    }
    fn spawn(&self, plan: &RunPlan) -> Result<ChildHandle> {
        self.log(&format!("spawn {}", plan.argv.join(" ")));
        Ok(ChildHandle {
            pid: 4242,
            start_ticks: 99,
        })
    }
    fn wait(
        &self,
        _: &ChildHandle,
        _: &RunPlan,
        _: &AtomicBool,
        emit: &dyn Fn(EventKind),
    ) -> Result<RunEnd> {
        self.log("wait");
        emit(EventKind::Log {
            stream: atlasctl_protocol::msg::bench_event::LogStream::Run,
            lines: vec!["  [  1.0s] warmup".into()],
        });
        Ok(self.end.clone())
    }
    fn collect(&self, _: &Path, _: &str, _: u64, _: &Path) -> Result<Vec<ArtifactMeta>> {
        self.log("collect");
        Ok(self.artifacts.clone())
    }
}

pub(super) struct World {
    pub(super) _dir: PathBuf,
    pub(super) store: JobStore,
    pub(super) journal: Journal,
    pub(super) job: JobRecord,
}

pub(super) fn world() -> World {
    let dir = std::env::temp_dir().join(format!(
        "bench-machine-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let store = JobStore::new(dir.join("jobs"));
    let key = JobKey::parse("k").unwrap();
    let sender = NodeId::parse(&"ef".repeat(32)).unwrap();
    let id = JobStore::mint_id(1, &key, sender);
    let job = JobRecord {
        id: id.clone(),
        key,
        sender,
        spec: JobSpec {
            sha: Sha::parse(&"ab".repeat(20)).unwrap(),
            gate: GateId::parse("decode-floor").unwrap(),
            params: BTreeMap::from([("osl".to_string(), "8".to_string())]),
            checkpoint: None,
            max_run_s: 600,
        },
        note: None,
        state: JobState::Queued,
        created_at_s: 1,
        updated_at_s: 1,
        child_pid: None,
        child_start_ticks: None,
        binary_sha256: None,
        seq_high: 0,
        outcome: None,
    };
    store.create(&job).unwrap();
    let journal = Journal::open(store.journal_path(&id), id).unwrap();
    World {
        _dir: dir,
        store,
        journal,
        job,
    }
}

pub(super) fn ctx<'a>(w: &'a World, ports: &'a dyn Ports, cancel: Arc<AtomicBool>) -> Ctx<'a> {
    Ctx {
        ports,
        store: &w.store,
        journal: &w.journal,
        local: NodeId::parse(&"11".repeat(32)).unwrap(),
        cancel,
        hardware: "gb10",
        child_env: vec![("ATLAS_HOME".into(), "/h".into())],
        build_timeout: Duration::from_secs(10),
        stall_timeout: Duration::from_secs(10),
        allow_unpublished: false,
    }
}

pub(super) fn kinds(w: &World) -> Vec<String> {
    w.journal
        .replay(1)
        .unwrap()
        .into_iter()
        .map(|e| {
            serde_json::to_value(&e.kind).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}
