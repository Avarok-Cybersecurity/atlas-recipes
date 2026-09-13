// SPDX-License-Identifier: AGPL-3.0-only

//! The per-job event journal: an append-only NDJSON file with a monotonic
//! seq, and a watch that wakes attached readers.
//!
//! The file is the truth. A reader that lost its connection replays from the
//! seq it last saw, so nothing is missed and nothing is delivered twice.

use anyhow::{Context, Result};
use atlasctl_protocol::msg::bench::JobId;
use atlasctl_protocol::msg::bench_event::{BenchEvent, EventKind};
use std::io::{BufRead, Seek, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// One job's journal.
#[derive(Debug, Clone)]
pub struct Journal {
    inner: Arc<Mutex<Inner>>,
    /// Latest journaled seq; attached readers wait on it.
    seq_tx: watch::Sender<u64>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    job: JobId,
    seq: u64,
}

impl Journal {
    /// Open (or create) the journal at `path`, resuming `seq` from what is
    /// already there.
    ///
    /// # Errors
    /// If an existing file cannot be read; a truncated last line is dropped
    /// rather than fatal.
    pub fn open(path: PathBuf, job: JobId) -> Result<Self> {
        let seq = replay(&path, 0)?.last().map_or(0, |e| e.seq);
        let (seq_tx, _) = watch::channel(seq);
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner { path, job, seq })),
            seq_tx,
        })
    }

    /// Append one event, assigning the next seq. `Heartbeat` is refused: it
    /// is never journaled.
    ///
    /// # Errors
    /// On I/O, or for a heartbeat.
    pub fn append(&self, kind: EventKind, at_ms: u64) -> Result<BenchEvent> {
        anyhow::ensure!(
            !matches!(kind, EventKind::Heartbeat { .. }),
            "heartbeats are never journaled"
        );
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let seq = inner.seq + 1;
        let event = BenchEvent {
            job: inner.job.clone(),
            seq,
            at_ms,
            kind,
        };
        let mut line = serde_json::to_vec(&event)?;
        line.push(b'\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&inner.path)
            .with_context(|| format!("opening {}", inner.path.display()))?;
        f.write_all(&line)?;
        if matches!(event.kind, EventKind::Done { .. }) {
            f.sync_all()?;
        }
        inner.seq = seq;
        // `send_replace`, not `send`: a watch sender with no receiver refuses
        // `send` and keeps the old value, which would leave `seq_high` at
        // zero until somebody attached.
        self.seq_tx.send_replace(seq);
        Ok(event)
    }

    /// The highest journaled seq.
    #[must_use]
    pub fn seq_high(&self) -> u64 {
        *self.seq_tx.borrow()
    }

    /// A receiver that changes whenever an event is appended.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.seq_tx.subscribe()
    }

    /// Every journaled event with `seq >= from`.
    ///
    /// # Errors
    /// On I/O.
    pub fn replay(&self, from: u64) -> Result<Vec<BenchEvent>> {
        let path = self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .path
            .clone();
        replay(&path, from)
    }
}

/// Read events from the file, tolerating a torn final line (an agent that
/// died mid-write).
fn replay(path: &std::path::Path, from: u64) -> Result<Vec<BenchEvent>> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("opening {}", path.display())),
    };
    let mut reader = std::io::BufReader::new(f);
    reader.rewind()?;
    let mut out = Vec::new();
    let mut lines = reader.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line?;
        match serde_json::from_str::<BenchEvent>(&line) {
            Ok(e) => {
                if e.seq >= from {
                    out.push(e);
                }
            }
            Err(_) if lines.peek().is_none() => break, // torn tail
            Err(e) => return Err(e).with_context(|| format!("{} is corrupt", path.display())),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlasctl_protocol::msg::bench::JobState;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "bench-journal-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p.join("events.ndjson")
    }

    fn job() -> JobId {
        JobId::parse("jb-1-deadbeef").unwrap()
    }

    #[test]
    fn appends_number_from_one_and_replay_resumes_from_any_seq() {
        let j = Journal::open(tmp(), job()).unwrap();
        assert_eq!(j.seq_high(), 0);
        let e1 = j.append(EventKind::Queued { position: 1 }, 10).unwrap();
        let e2 = j
            .append(
                EventKind::Progress {
                    phase: "warmup".into(),
                    detail: String::new(),
                },
                20,
            )
            .unwrap();
        assert_eq!((e1.seq, e2.seq), (1, 2));
        assert_eq!(j.seq_high(), 2);
        assert_eq!(j.replay(1).unwrap().len(), 2);
        assert_eq!(j.replay(2).unwrap(), vec![e2.clone()]);
        assert!(j.replay(3).unwrap().is_empty());
        // Reopening resumes the counter from the file, not from zero.
        let path = j.inner.lock().unwrap().path.clone();
        let again = Journal::open(path, job()).unwrap();
        assert_eq!(again.seq_high(), 2);
        assert_eq!(
            again
                .append(EventKind::Queued { position: 0 }, 30)
                .unwrap()
                .seq,
            3
        );
    }

    /// NEGATIVE CONTROL: a heartbeat never enters the journal.
    #[test]
    fn heartbeats_are_refused() {
        let j = Journal::open(tmp(), job()).unwrap();
        assert!(
            j.append(
                EventKind::Heartbeat {
                    state: JobState::Running,
                    seq_high: 0
                },
                1
            )
            .is_err()
        );
        assert_eq!(j.seq_high(), 0);
    }

    #[test]
    fn a_torn_last_line_is_dropped_and_the_watch_wakes_on_append() {
        let path = tmp();
        let j = Journal::open(path.clone(), job()).unwrap();
        j.append(EventKind::Queued { position: 1 }, 1).unwrap();
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(b"{\"job\":\"jb-1-deadbeef\",\"seq\":2,\"at")
                .unwrap();
        }
        let again = Journal::open(path, job()).unwrap();
        assert_eq!(again.seq_high(), 1, "the torn tail does not count");
        let mut rx = again.watch();
        assert_eq!(*rx.borrow_and_update(), 1);
        again.append(EventKind::Queued { position: 0 }, 2).unwrap();
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), 2);
    }
}
