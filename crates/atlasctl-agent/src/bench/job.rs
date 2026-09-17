// SPDX-License-Identifier: AGPL-3.0-only

//! Jobs on disk: the record, the key index, and the directory each job owns.
//!
//! Every job is a directory under `<cache>/jobs/<id>/` holding `job.json`
//! (the record, rewritten atomically on every transition), `events.ndjson`
//! (the journal), `child.log` (the child's stdio, a file so it survives an
//! agent restart) and `artifacts/`. The job store is the source of truth: an
//! attached client is a view of it, and a restart rebuilds the queue from it.

use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::{GateId, JobId, JobKey, JobState, JobSummary, Sha};
use atlasctl_protocol::msg::bench_event::Outcome;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a submission asked for. Two submissions with one key must agree on
/// all of this, or the second is a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub sha: Sha,
    pub gate: GateId,
    pub params: BTreeMap<String, String>,
    pub checkpoint: Option<String>,
    pub max_run_s: u32,
}

/// The record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: JobId,
    pub key: JobKey,
    pub sender: NodeId,
    pub spec: JobSpec,
    pub note: Option<String>,
    pub state: JobState,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    /// The child's pid and its `/proc/<pid>/stat` start ticks while it runs,
    /// so a restarted agent can tell the same process from a reused pid.
    pub child_pid: Option<u32>,
    pub child_start_ticks: Option<u64>,
    /// The commit's built binary, once known.
    pub binary_sha256: Option<String>,
    /// What the gate's record directory held the moment before the child
    /// started. Persisted because the attribution has to survive an agent
    /// restart: `resume` has no other way to tell this job's record from one a
    /// previous job left in the same worktree.
    #[serde(default)]
    pub records_before: crate::bench::ports::RecordState,
    pub seq_high: u64,
    pub outcome: Option<Outcome>,
}

impl JobRecord {
    #[must_use]
    pub fn summary(&self) -> JobSummary {
        JobSummary {
            job: self.id.clone(),
            job_key: self.key.clone(),
            sha: self.spec.sha.clone(),
            gate: self.spec.gate.clone(),
            state: self.state,
            created_at_s: self.created_at_s,
            updated_at_s: self.updated_at_s,
            seq_high: self.seq_high,
            outcome: self.outcome.clone(),
        }
    }
}

/// The store.
#[derive(Debug, Clone)]
pub struct JobStore {
    root: PathBuf,
}

impl JobStore {
    pub fn new(jobs_dir: PathBuf) -> Self {
        Self { root: jobs_dir }
    }

    pub fn dir(&self, id: &JobId) -> PathBuf {
        self.root.join(id.as_str())
    }
    pub fn record_path(&self, id: &JobId) -> PathBuf {
        self.dir(id).join("job.json")
    }
    pub fn journal_path(&self, id: &JobId) -> PathBuf {
        self.dir(id).join("events.ndjson")
    }
    pub fn child_log_path(&self, id: &JobId) -> PathBuf {
        self.dir(id).join("child.log")
    }
    pub fn artifacts_dir(&self, id: &JobId) -> PathBuf {
        self.dir(id).join("artifacts")
    }
    fn key_path(&self, key: &JobKey) -> PathBuf {
        self.root.join("by-key").join(key.as_str())
    }

    /// Mint an id: `jb-<unix>-<8 hex>` from the clock and the key.
    #[must_use]
    pub fn mint_id(now_s: u64, key: &JobKey, sender: NodeId) -> JobId {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(now_s.to_le_bytes());
        h.update(key.as_str().as_bytes());
        h.update(sender.to_string().as_bytes());
        let digest = h.finalize();
        JobId::parse(&format!("jb-{now_s}-{}", hex::encode(&digest[..4]))).expect("shape is fixed")
    }

    /// The job a key already names, if any.
    ///
    /// # Errors
    /// If the index or the record cannot be read.
    pub fn by_key(&self, key: &JobKey) -> Result<Option<JobRecord>> {
        let path = self.key_path(key);
        let id = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let id = JobId::parse(id.trim()).context("the key index names a malformed job id")?;
        self.load(&id).map(Some)
    }

    /// Create a job directory and its record, and index its key.
    ///
    /// # Errors
    /// If the directory exists already (ids are minted from the clock and
    /// the key; a collision is a bug, not a race to paper over), or on I/O.
    pub fn create(&self, record: &JobRecord) -> Result<()> {
        let dir = self.dir(&record.id);
        if dir.exists() {
            bail!("job directory {} already exists", dir.display());
        }
        std::fs::create_dir_all(dir.join("artifacts"))
            .with_context(|| format!("creating {}", dir.display()))?;
        self.save(record)?;
        let key_path = self.key_path(&record.key);
        std::fs::create_dir_all(key_path.parent().expect("has a parent"))?;
        write_atomic(&key_path, record.id.as_str().as_bytes())?;
        Ok(())
    }

    /// Rewrite the record atomically.
    ///
    /// # Errors
    /// On I/O.
    pub fn save(&self, record: &JobRecord) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(record)?;
        write_atomic(&self.record_path(&record.id), &bytes)
    }

    /// # Errors
    /// If there is no such job or its record is unreadable.
    pub fn load(&self, id: &JobId) -> Result<JobRecord> {
        let path = self.record_path(id);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("no job {id} ({})", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("{} is malformed", path.display()))
    }

    /// Every job, newest first.
    ///
    /// # Errors
    /// On I/O.
    pub fn list(&self) -> Result<Vec<JobRecord>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e).with_context(|| format!("listing {}", self.root.display())),
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Ok(id) = JobId::parse(&name.to_string_lossy()) else {
                continue;
            };
            if let Ok(r) = self.load(&id) {
                out.push(r);
            }
        }
        out.sort_by(|a, b| b.created_at_s.cmp(&a.created_at_s).then(b.id.cmp(&a.id)));
        Ok(out)
    }

    /// Remove a terminal job and its key index entry.
    ///
    /// # Errors
    /// If the job is not terminal, or on I/O.
    pub fn remove(&self, record: &JobRecord) -> Result<()> {
        if !record.state.is_terminal() {
            bail!(
                "refusing to remove {} in state {:?}",
                record.id,
                record.state
            );
        }
        let _ = std::fs::remove_file(self.key_path(&record.key));
        std::fs::remove_dir_all(self.dir(&record.id))
            .with_context(|| format!("removing job {}", record.id))
    }
}

/// Write via a sibling temp file and rename, so a reader never sees half.
///
/// # Errors
/// On I/O.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))
}

pub fn now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> JobStore {
        let p = std::env::temp_dir().join(format!(
            "bench-jobs-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        JobStore::new(p)
    }

    fn sender() -> NodeId {
        NodeId::parse(&"cd".repeat(32)).unwrap()
    }

    fn record(key: &str, now: u64) -> JobRecord {
        let key = JobKey::parse(key).unwrap();
        JobRecord {
            id: JobStore::mint_id(now, &key, sender()),
            key,
            sender: sender(),
            spec: JobSpec {
                sha: Sha::parse(&"ab".repeat(20)).unwrap(),
                gate: GateId::parse("decode-floor").unwrap(),
                params: BTreeMap::new(),
                checkpoint: None,
                max_run_s: 600,
            },
            note: None,
            state: JobState::Queued,
            created_at_s: now,
            updated_at_s: now,
            child_pid: None,
            child_start_ticks: None,
            binary_sha256: None,
            records_before: Default::default(),
            seq_high: 0,
            outcome: None,
        }
    }

    #[test]
    fn create_load_index_and_list_round_trip() {
        let s = store();
        let a = record("k-a", 100);
        let b = record("k-b", 200);
        s.create(&a).unwrap();
        s.create(&b).unwrap();
        assert_eq!(s.load(&a.id).unwrap(), a);
        assert_eq!(s.by_key(&a.key).unwrap().unwrap().id, a.id);
        assert!(s.by_key(&JobKey::parse("k-c").unwrap()).unwrap().is_none());
        let ids: Vec<JobId> = s.list().unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![b.id.clone(), a.id.clone()], "newest first");
        assert!(s.dir(&a.id).join("artifacts").is_dir());
    }

    #[test]
    fn ids_are_stable_for_one_input_and_distinct_across_keys() {
        let k = JobKey::parse("x").unwrap();
        assert_eq!(
            JobStore::mint_id(5, &k, sender()),
            JobStore::mint_id(5, &k, sender())
        );
        assert_ne!(
            JobStore::mint_id(5, &k, sender()),
            JobStore::mint_id(5, &JobKey::parse("y").unwrap(), sender())
        );
        assert!(
            JobStore::mint_id(5, &k, sender())
                .as_str()
                .starts_with("jb-5-")
        );
    }

    /// NEGATIVE CONTROLS: a live job cannot be removed; a duplicate create
    /// is refused rather than clobbering.
    #[test]
    fn only_terminal_jobs_are_removable_and_creates_never_clobber() {
        let s = store();
        let mut a = record("k-a", 100);
        s.create(&a).unwrap();
        assert!(s.remove(&a).is_err());
        assert!(s.create(&a).is_err());
        a.state = JobState::Done;
        s.save(&a).unwrap();
        s.remove(&a).unwrap();
        assert!(s.load(&a.id).is_err());
        assert!(s.by_key(&a.key).unwrap().is_none());
    }
}
