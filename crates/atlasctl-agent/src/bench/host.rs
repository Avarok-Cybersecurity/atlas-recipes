// SPDX-License-Identifier: AGPL-3.0-only

//! What the peer-serving path calls: submit, status, cancel, artifacts.
//!
//! One `BenchHost` per agent. It owns the job store and the single worker's
//! queue; the worker ([`super::runner`]) takes jobs from it and reports
//! back through it. Every method answers with a [`BenchRep`] — including
//! refusals, which are answers, not errors — so the serving path has one
//! shape to write back.

use super::config::BenchConfig;
use super::exclusive;
use super::job::{JobRecord, JobSpec, JobStore, now_s};
use super::journal::Journal;
use anyhow::{Context, Result};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::{
    BenchRefusal, BenchRep, GateId, JobId, JobKey, JobState, MAX_CHUNK, MAX_NOTE, Sha, check_params,
};
use atlasctl_protocol::msg::bench_event::ArtifactMeta;
use base64::Engine as _;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Submissions one peer may make per minute.
pub const SUBMITS_PER_MINUTE: usize = 30;

pub struct BenchHost {
    pub cfg: BenchConfig,
    pub store: JobStore,
    pub local: NodeId,
    pub fleet: Arc<crate::fleet::LocalFleet>,
    /// Open journals: every job that is running or was attached recently.
    journals: Mutex<HashMap<JobId, Journal>>,
    /// Cancel flags of jobs the worker is executing.
    cancels: Mutex<HashMap<JobId, Arc<AtomicBool>>>,
    running: Mutex<Option<JobId>>,
    submits: Mutex<HashMap<NodeId, VecDeque<Instant>>>,
    /// Woken by `submit`; the worker sleeps on it.
    pub wake: tokio::sync::Notify,
}

impl BenchHost {
    pub fn new(
        cfg: BenchConfig,
        local: NodeId,
        fleet: Arc<crate::fleet::LocalFleet>,
    ) -> Result<Arc<Self>> {
        std::fs::create_dir_all(cfg.jobs_dir())
            .with_context(|| format!("creating {}", cfg.jobs_dir().display()))?;
        Ok(Arc::new(Self {
            store: JobStore::new(cfg.jobs_dir()),
            cfg,
            local,
            fleet,
            journals: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            running: Mutex::new(None),
            submits: Mutex::new(HashMap::new()),
            wake: tokio::sync::Notify::new(),
        }))
    }

    fn refuse(&self, refusal: BenchRefusal) -> BenchRep {
        BenchRep::Refused {
            by: self.local,
            refusal,
        }
    }

    /// The journal for `job`, opened on demand and kept.
    pub fn journal(&self, job: &JobId) -> Result<Journal> {
        let mut j = self.journals.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = j.get(job) {
            return Ok(existing.clone());
        }
        let opened = Journal::open(self.store.journal_path(job), job.clone())?;
        j.insert(job.clone(), opened.clone());
        Ok(opened)
    }

    pub fn running(&self) -> Option<JobId> {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub(super) fn set_running(&self, job: Option<JobId>) {
        *self.running.lock().unwrap_or_else(|p| p.into_inner()) = job;
    }

    pub(super) fn register_cancel(&self, job: &JobId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(job.clone(), flag.clone());
        flag
    }

    pub(super) fn forget_cancel(&self, job: &JobId) {
        self.cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(job);
        self.journals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(job);
    }

    fn rate_limited(&self, sender: NodeId) -> Option<u32> {
        let mut m = self.submits.lock().unwrap_or_else(|p| p.into_inner());
        let q = m.entry(sender).or_default();
        let now = Instant::now();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
        {
            q.pop_front();
        }
        if q.len() >= SUBMITS_PER_MINUTE {
            return Some(60);
        }
        q.push_back(now);
        None
    }

    /// Queued jobs, oldest first.
    pub fn queued(&self) -> Result<Vec<JobRecord>> {
        let mut q: Vec<JobRecord> = self
            .store
            .list()?
            .into_iter()
            .filter(|j| j.state == JobState::Queued)
            .collect();
        q.sort_by(|a, b| a.created_at_s.cmp(&b.created_at_s).then(a.id.cmp(&b.id)));
        Ok(q)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &self,
        sender: NodeId,
        job_key: JobKey,
        sha: Sha,
        gate: GateId,
        params: BTreeMap<String, String>,
        checkpoint: Option<String>,
        hardware: Option<String>,
        max_run_s: Option<u32>,
        note: Option<String>,
    ) -> BenchRep {
        if let Some(retry) = self.rate_limited(sender) {
            return self.refuse(BenchRefusal::RateLimited {
                retry_after_s: retry,
            });
        }
        if let Err(e) = check_params(&params) {
            return self.refuse(BenchRefusal::BadParams { errors: vec![e] });
        }
        if let Some(c) = &checkpoint
            && (c.is_empty()
                || c.len() > 128
                || !c
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || "._/-".contains(ch)))
        {
            return self.refuse(BenchRefusal::BadParams {
                errors: vec![format!("checkpoint {c:?} is not a checkpoint name")],
            });
        }
        if let Some(h) = &hardware
            && *h != self.cfg.hardware
        {
            return self.refuse(BenchRefusal::NotConfigured {
                what: format!("this node's box class is {}, not {h}", self.cfg.hardware),
                fix: "submit for the class the node is, or pick another node".into(),
            });
        }
        let note = note.map(|n| {
            n.chars()
                .filter(|c| !c.is_control())
                .take(MAX_NOTE)
                .collect::<String>()
        });
        let spec = JobSpec {
            sha,
            gate,
            params,
            checkpoint,
            max_run_s: max_run_s.map_or(self.cfg.max_run_s, |m| m.min(self.cfg.max_run_s)),
        };
        match self.store.by_key(&job_key) {
            Ok(Some(existing)) => {
                if existing.spec == spec {
                    return BenchRep::Accepted {
                        position: self.position_of(&existing.id),
                        job: existing.id,
                        existing: true,
                        state: existing.state,
                    };
                }
                return self.refuse(BenchRefusal::KeyConflict { job: existing.id });
            }
            Ok(None) => {}
            Err(e) => {
                return self.refuse(BenchRefusal::NotConfigured {
                    what: format!("the job store is unreadable: {e:#}"),
                    fix: "check the cache directory's permissions".into(),
                });
            }
        }
        let depth = match self.queued() {
            Ok(q) => q.len() as u32,
            Err(_) => u32::MAX,
        };
        if depth >= self.cfg.queue_depth {
            return self.refuse(BenchRefusal::QueueFull {
                depth,
                retry_after_s: 300,
            });
        }
        // Disk is checked at submit: a job that cannot even be journaled
        // should not be accepted. Memory and the GPU are checked by the
        // worker right before it starts the child, because they change.
        let readings = exclusive::probe(&self.cfg.cache_dir, None, &self.cfg.atlas_home);
        if let Some(free) = readings.disk_free_bytes
            && free < self.cfg.min_free_disk_bytes
        {
            return self.refuse(BenchRefusal::DiskLow {
                free_bytes: free,
                required_bytes: self.cfg.min_free_disk_bytes,
            });
        }
        let now = now_s();
        let id = JobStore::mint_id(now, &job_key, sender);
        let record = JobRecord {
            id: id.clone(),
            key: job_key,
            sender,
            spec,
            note,
            state: JobState::Queued,
            created_at_s: now,
            updated_at_s: now,
            child_pid: None,
            child_start_ticks: None,
            binary_sha256: None,
            records_before: Default::default(),
            seq_high: 0,
            outcome: None,
        };
        if let Err(e) = self.store.create(&record) {
            return self.refuse(BenchRefusal::NotConfigured {
                what: format!("cannot create the job: {e:#}"),
                fix: "check the cache directory".into(),
            });
        }
        if let Ok(j) = self.journal(&id) {
            let _ = j.append(
                atlasctl_protocol::msg::bench_event::EventKind::Queued {
                    position: depth + 1,
                },
                now * 1000,
            );
        }
        self.wake.notify_one();
        BenchRep::Accepted {
            job: id,
            existing: false,
            state: JobState::Queued,
            position: depth + 1,
        }
    }

    fn position_of(&self, job: &JobId) -> u32 {
        self.queued()
            .ok()
            .and_then(|q| q.iter().position(|j| j.id == *job))
            .map_or(0, |p| p as u32 + 1)
    }

    pub fn status(&self, job: Option<JobId>) -> BenchRep {
        match job {
            Some(id) => match self.store.load(&id) {
                Ok(r) => BenchRep::Status {
                    jobs: vec![r.summary()],
                },
                Err(_) => self.refuse(BenchRefusal::UnknownJob { job: id }),
            },
            None => BenchRep::Status {
                jobs: self
                    .store
                    .list()
                    .unwrap_or_default()
                    .iter()
                    .take(50)
                    .map(JobRecord::summary)
                    .collect(),
            },
        }
    }

    /// Idempotent: a terminal job answers with its state; a queued job is
    /// cancelled here; a running one is signalled to its worker.
    pub fn cancel(&self, job: JobId) -> BenchRep {
        let Ok(mut record) = self.store.load(&job) else {
            return self.refuse(BenchRefusal::UnknownJob { job });
        };
        if record.state.is_terminal() {
            return BenchRep::Cancelled {
                job,
                state: record.state,
            };
        }
        if record.state == JobState::Queued {
            record.outcome =
                Some(atlasctl_protocol::msg::bench_event::Outcome::Cancelled { by: self.local });
            record.state = JobState::Done;
            record.updated_at_s = now_s();
            if let Ok(j) = self.journal(&job) {
                let _ = j.append(
                    atlasctl_protocol::msg::bench_event::EventKind::Done {
                        outcome: atlasctl_protocol::msg::bench_event::Outcome::Cancelled {
                            by: self.local,
                        },
                    },
                    now_s() * 1000,
                );
                record.seq_high = j.seq_high();
            }
            let _ = self.store.save(&record);
            return BenchRep::Cancelled {
                job,
                state: JobState::Done,
            };
        }
        if let Some(flag) = self
            .cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&job)
        {
            flag.store(true, Ordering::SeqCst);
        }
        BenchRep::Cancelled {
            job,
            state: record.state,
        }
    }

    pub fn artifacts(&self, job: JobId) -> BenchRep {
        match self.artifact_list(&job) {
            Ok(items) => BenchRep::Artifacts { job, items },
            Err(_) => self.refuse(BenchRefusal::UnknownJob { job }),
        }
    }

    fn artifact_list(&self, job: &JobId) -> Result<Vec<ArtifactMeta>> {
        let record = self.store.load(job)?;
        let journal = self.journal(job)?;
        let mut items: Vec<ArtifactMeta> = journal
            .replay(1)?
            .into_iter()
            .filter_map(|e| match e.kind {
                atlasctl_protocol::msg::bench_event::EventKind::Artifact { meta } => Some(meta),
                _ => None,
            })
            .collect();
        // The child's log is always offered, named after the job.
        let log = self.store.child_log_path(job);
        if let Ok(meta) = std::fs::metadata(&log)
            && let Ok((sha256, _)) = super::ports_std::sha256_file(&log)
        {
            items.push(ArtifactMeta {
                name: "child.log".into(),
                relative_path: format!(
                    ".certify/{}/{}.log",
                    record.spec.sha.short(),
                    record.spec.gate
                ),
                bytes: meta.len(),
                sha256,
                kind: atlasctl_protocol::msg::bench_event::ArtifactKind::RunLog,
            });
        }
        Ok(items)
    }

    pub fn chunk(&self, job: JobId, name: String, offset: u64, len: u32) -> BenchRep {
        use std::io::{Read, Seek};
        // A bare file name from the job's own manifest, never a path.
        if name.contains('/') || name.contains('\\') || name.starts_with('.') && name != ".sig" {
            return self.refuse(BenchRefusal::NoSuchArtifact { name });
        }
        let Ok(items) = self.artifact_list(&job) else {
            return self.refuse(BenchRefusal::UnknownJob { job });
        };
        if !items.iter().any(|a| a.name == name) {
            return self.refuse(BenchRefusal::NoSuchArtifact { name });
        }
        let path = if name == "child.log" {
            self.store.child_log_path(&job)
        } else {
            self.store.artifacts_dir(&job).join(&name)
        };
        let Ok(mut f) = std::fs::File::open(&path) else {
            return self.refuse(BenchRefusal::NoSuchArtifact { name });
        };
        let total = f.metadata().map_or(0, |m| m.len());
        let want = len.min(MAX_CHUNK) as usize;
        let mut buf = vec![0u8; want];
        let n = f
            .seek(std::io::SeekFrom::Start(offset))
            .and_then(|_| f.read(&mut buf))
            .unwrap_or(0);
        buf.truncate(n);
        BenchRep::Chunk {
            job,
            name,
            offset,
            data_b64: base64::engine::general_purpose::STANDARD.encode(&buf),
            eof: offset + n as u64 >= total,
        }
    }
}
