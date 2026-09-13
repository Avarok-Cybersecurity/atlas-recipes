// SPDX-License-Identifier: AGPL-3.0-only

//! Bringing a job's files back, byte-exact.
//!
//! The node names each artifact with a repo-relative path and a sha256; this
//! side downloads in chunks, hashes what arrived, and writes only a file that
//! matches. A path that would land outside `--out-dir` is refused before a
//! byte is requested — the node is trusted to run a gate, not to choose
//! where files go on this machine.

use super::address::Target;
use super::exit::BenchError;
use super::session::{Reached, Session};
use atlasctl_protocol::msg::bench::{JobId, MAX_CHUNK};
use atlasctl_protocol::msg::bench_event::ArtifactMeta;
use atlasctl_protocol::msg::{BenchRep, BenchReq};
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One artifact, on disk.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Fetched {
    pub name: String,
    pub relative_path: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// The place an artifact may be written, or why not.
///
/// # Errors
/// `bad_args` for an absolute path, a `..`, an empty path, or a path whose
/// last component is not the artifact's own name.
pub fn destination(out_dir: &Path, meta: &ArtifactMeta) -> Result<PathBuf, BenchError> {
    let rel = &meta.relative_path;
    if rel.is_empty() {
        return Err(BenchError::bad_args(format!(
            "artifact {} has an empty path",
            meta.name
        )));
    }
    // Checked on the text, not on `Path::components`, which folds a `.` away
    // and would let `.benchmarks/./x` through as tidy.
    let plain = !rel.starts_with('/')
        && !rel.contains('\\')
        && rel
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..");
    if !plain {
        return Err(BenchError::bad_args(format!(
            "artifact {} names {rel:?}, which is not a plain relative path",
            meta.name
        )));
    }
    let rel = Path::new(rel);
    if rel.file_name().and_then(|f| f.to_str()) != Some(meta.name.as_str()) {
        return Err(BenchError::bad_args(format!(
            "artifact {} would be written as {:?}; the file name must match",
            meta.name, meta.relative_path
        )));
    }
    Ok(out_dir.join(rel))
}

/// Check what arrived against what was promised.
///
/// # Errors
/// `io` naming the mismatch.
pub fn verify(meta: &ArtifactMeta, data: &[u8]) -> Result<(), BenchError> {
    if data.len() as u64 != meta.bytes {
        return Err(BenchError::io(format!(
            "artifact {}: {} bytes arrived, {} promised",
            meta.name,
            data.len(),
            meta.bytes
        )));
    }
    let got = hex::encode(Sha256::digest(data));
    if got != meta.sha256 {
        return Err(BenchError::io(format!(
            "artifact {}: sha256 {got} arrived, {} promised",
            meta.name, meta.sha256
        )));
    }
    Ok(())
}

/// List a job's artifacts.
///
/// # Errors
/// A refusal (unknown job), or a link failure.
pub fn list(
    session: &Session,
    target: &Target,
    addrs: &[std::net::SocketAddr],
    job: &JobId,
) -> Result<(Reached, Vec<ArtifactMeta>), BenchError> {
    let (reached, rep) =
        session.request(target, addrs, &BenchReq::ArtifactList { job: job.clone() })?;
    match rep {
        BenchRep::Artifacts { items, .. } => Ok((reached, items)),
        other => Err(BenchError::unexpected(&target.given, &other)),
    }
}

/// Download one artifact fully into memory, chunk by chunk.
fn download(
    session: &Session,
    target: &Target,
    addr: std::net::SocketAddr,
    job: &JobId,
    meta: &ArtifactMeta,
) -> Result<Vec<u8>, BenchError> {
    let mut data = Vec::with_capacity(usize::try_from(meta.bytes).unwrap_or(0));
    let engine = base64::engine::general_purpose::STANDARD;
    loop {
        let offset = data.len() as u64;
        let (_, rep) = session.request(
            target,
            &[addr],
            &BenchReq::ArtifactChunk {
                job: job.clone(),
                name: meta.name.clone(),
                offset,
                len: MAX_CHUNK,
            },
        )?;
        let BenchRep::Chunk {
            offset: got_off,
            data_b64,
            eof,
            ..
        } = rep
        else {
            return Err(BenchError::unexpected(&target.given, &rep));
        };
        if got_off != offset {
            return Err(BenchError::io(format!(
                "artifact {}: asked for offset {offset}, got {got_off}",
                meta.name
            ))
            .at(&target.given));
        }
        let chunk = engine
            .decode(data_b64.as_bytes())
            .map_err(|e| BenchError::io(format!("artifact {}: bad base64: {e}", meta.name)))?;
        data.extend_from_slice(&chunk);
        if data.len() as u64 > meta.bytes {
            return Err(BenchError::io(format!(
                "artifact {}: more than the {} bytes promised",
                meta.name, meta.bytes
            )));
        }
        if eof {
            return Ok(data);
        }
        if chunk.is_empty() {
            return Err(BenchError::io(format!(
                "artifact {}: empty chunk before eof at offset {offset}",
                meta.name
            )));
        }
    }
}

/// Download every artifact of `job` into `out_dir`, verified.
///
/// Files are written with `create_new`, so a second fetch into the same
/// directory refuses rather than overwrites — a record on disk is evidence.
///
/// # Errors
/// Any refusal, link or verification failure; nothing partial is left
/// behind for the file that failed.
pub fn fetch_all(
    session: &Session,
    target: &Target,
    addrs: &[std::net::SocketAddr],
    job: &JobId,
    out_dir: &Path,
) -> Result<(Reached, Vec<Fetched>), BenchError> {
    let (reached, items) = list(session, target, addrs, job)?;
    let mut out = Vec::with_capacity(items.len());
    for meta in &items {
        let dest = destination(out_dir, meta)?;
        let data = download(session, target, reached.addr, job, meta)?;
        verify(meta, &data)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BenchError::io(format!("creating {}: {e}", parent.display())))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|e| BenchError::io(format!("writing {}: {e}", dest.display())))?;
        std::io::Write::write_all(&mut f, &data)
            .map_err(|e| BenchError::io(format!("writing {}: {e}", dest.display())))?;
        out.push(Fetched {
            name: meta.name.clone(),
            relative_path: meta.relative_path.clone(),
            path: dest,
            bytes: meta.bytes,
            sha256: meta.sha256.clone(),
        });
    }
    Ok((reached, out))
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod fetch_tests;
