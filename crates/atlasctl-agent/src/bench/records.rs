// SPDX-License-Identifier: AGPL-3.0-only

//! Which records did THIS job write?
//!
//! Split from `ports_std` because it answers a question of its own, and because
//! answering it wrongly is expensive: on 2026-09-17 the previous job's record
//! was handed back with this one's, the submitter refused the pair, and a
//! 27-minute shard was lost. The answer is a difference against a snapshot
//! taken immediately before the child starts — never "everything modified since
//! a wall-clock instant".

use std::path::Path;

use anyhow::Result;
use atlasctl_protocol::msg::bench_event::{ArtifactKind, ArtifactMeta};

use super::ports::RecordState;
use super::ports_std::sha256_file;

/// Every file in a gate's record directory, with the nanosecond it was last
/// written. Nanoseconds rather than seconds because two writes inside one
/// second are exactly the case this exists to tell apart.
pub(super) fn state(dir: &Path) -> RecordState {
    let mut out = RecordState::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") && !name.ends_with(".json.sig") {
            continue;
        }
        let written = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        out.insert(name, written);
    }
    out
}

/// The records this job wrote: those absent from `before`, and those whose
/// contents were rewritten since it was taken.
pub(super) fn collect(
    worktree: &Path,
    gate: &str,
    before: &RecordState,
    dest: &Path,
) -> Result<Vec<ArtifactMeta>> {
    std::fs::create_dir_all(dest)?;
    let mut out = Vec::new();
    let dir = worktree.join(".benchmarks").join(gate);
    for (name, written) in state(&dir) {
        // Absent before, or written again since: either way this job wrote it.
        // A file the previous job left behind has the timestamp it had in the
        // snapshot, so it is not attributed here however close the two ran.
        if before.get(&name) == Some(&written) {
            continue;
        }
        let kind = if name.ends_with(".json.sig") {
            ArtifactKind::Signature
        } else if name.ends_with(".json") {
            ArtifactKind::Record
        } else {
            continue;
        };
        let target = dest.join(&name);
        std::fs::copy(dir.join(&name), &target)?;
        let (sha256, bytes) = sha256_file(&target)?;
        out.push(ArtifactMeta {
            relative_path: format!(".benchmarks/{gate}/{name}"),
            name,
            bytes,
            sha256,
            kind,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
#[path = "records_tests.rs"]
mod records_tests;
