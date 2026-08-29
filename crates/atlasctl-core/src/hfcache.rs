// SPDX-License-Identifier: AGPL-3.0-only

//! Where a Hub model lives in a local HuggingFace cache.
//!
//! A launch mounts the host cache at `/cache/huggingface` and sets
//! `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1` (see `docker::translate`),
//! deliberately: a recipe must not reach the network mid-launch. The
//! consequence is that a model which is not already in that cache cannot be
//! fetched by the container — it fails inside, after the image pull, with the
//! Hub library's own cache-miss message. This module exists so the launcher can
//! say so first, in terms of the recipe the operator named.

use std::path::{Path, PathBuf};

/// The directory a Hub id `org/name` occupies under an HF cache root.
///
/// `None` when `model` is not a two-part Hub id — a bare name, a local path, or
/// anything with extra slashes. Those do not follow this layout, so the
/// caller must not conclude anything about them from a missing directory.
pub fn hub_dir(cache_root: &Path, model: &str) -> Option<PathBuf> {
    // A path, not an id: `/models/foo`, `./foo`, `~/foo`.
    if model.starts_with('/') || model.starts_with('.') || model.starts_with('~') {
        return None;
    }
    let mut parts = model.split('/');
    let (Some(org), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if org.is_empty() || name.is_empty() {
        return None;
    }
    Some(
        cache_root
            .join("hub")
            .join(format!("models--{org}--{name}")),
    )
}

/// What a Hub cache holds for a model.
///
/// A launch runs offline, so "the directory is there" is not the question the
/// launcher needs answered — "are the weights there" is. These are genuinely
/// different states, and the difference is common: a cache entry left by a
/// metadata-only fetch, an aborted download, or a cleanup that reclaimed blobs
/// keeps `refs/`, `snapshots/` and a resolvable `config.json` while every weight
/// file is gone. Nine such directories, 32-68 KB each, were sitting in the cache
/// of the machine this was written on — each one of which `Path::exists`
/// reported as a present model, sending the launch on to fail inside a container
/// with the Hub library's own cache-miss message.
#[derive(Debug, PartialEq, Eq)]
pub enum CacheState {
    /// No directory for this model at all.
    Absent,
    /// The directory exists but carries no usable weight file.
    MetadataOnly,
    /// At least one weight file is present and its blob resolves.
    Weights,
}

/// File extensions that constitute model weights.
///
/// Deliberately short: an extension listed here that is NOT weights would make a
/// metadata-only cache read as complete, which is the exact bug this exists to
/// prevent.
const WEIGHT_EXTENSIONS: [&str; 5] = ["safetensors", "bin", "gguf", "pt", "pth"];

/// How deep to look inside a snapshot for weights.
///
/// Every checkpoint in the cache this was written against is flat — weights sit
/// directly in the snapshot — but a Hub repo may nest them, and the two possible
/// mistakes here are not symmetric. Missing a weight file REFUSES a launch that
/// would have worked, which is the failure the operator cannot argue with.
/// Finding one the engine then cannot use costs nothing new: that is exactly
/// what happened before this check existed.
///
/// Bounded rather than unbounded so a cache with something enormous mounted
/// under it cannot turn a preflight check into a directory walk.
const MAX_SNAPSHOT_DEPTH: usize = 3;

/// Classify what `dir` (from [`hub_dir`]) actually holds.
pub fn cache_state(dir: &Path) -> CacheState {
    if !dir.exists() {
        return CacheState::Absent;
    }
    let Ok(snapshots) = std::fs::read_dir(dir.join("snapshots")) else {
        return CacheState::MetadataOnly;
    };
    // Any snapshot with weights is enough: a cache holding one complete
    // revision and one interrupted revision can serve the complete one.
    for snap in snapshots.flatten() {
        if has_weight_below(&snap.path(), MAX_SNAPSHOT_DEPTH) {
            return CacheState::Weights;
        }
    }
    CacheState::MetadataOnly
}

/// Whether a weight file that can actually be read exists at or below `dir`.
fn has_weight_below(dir: &Path, depth: usize) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // `file_type` does NOT follow symlinks, so a link is examined as a file
        // below rather than being descended into — which also means a cache
        // containing a symlink loop cannot spin here.
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            dirs.push(path);
            continue;
        }
        let is_weight = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| WEIGHT_EXTENSIONS.contains(&e));
        // `metadata` DOES follow the link into `blobs/`, so an entry whose blob
        // was reclaimed reads as absent rather than as a weight file. A dangling
        // link is exactly what a partially cleaned cache leaves behind.
        if is_weight && std::fs::metadata(&path).is_ok() {
            return true;
        }
    }
    if depth == 0 {
        return false;
    }
    dirs.iter().any(|d| has_weight_below(d, depth - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hub_id_maps_to_the_directory_the_hub_library_uses() {
        let root = Path::new("/c");
        assert_eq!(
            hub_dir(root, "unsloth/Qwen3.8-27B-NVFP4").unwrap(),
            Path::new("/c/hub/models--unsloth--Qwen3.8-27B-NVFP4")
        );
        // Dots and dashes in either half survive verbatim; only `/` is special.
        assert_eq!(
            hub_dir(root, "Qwen/Qwen3.6-35B-A3B-FP8").unwrap(),
            Path::new("/c/hub/models--Qwen--Qwen3.6-35B-A3B-FP8")
        );
    }

    /// The caller treats `None` as "cannot tell", so anything that does not
    /// follow the layout must return it rather than a path that will be absent
    /// and read as a missing model.
    #[test]
    fn anything_that_is_not_a_two_part_id_is_not_guessed_at() {
        let root = Path::new("/c");
        for not_an_id in [
            "/models/local",   // absolute path
            "./relative",      // relative path
            "~/home-relative", // home-relative
            "bare-name",       // no org
            "a/b/c",           // too many parts
            "/",               // degenerate
            "",                // empty
            "org/",            // empty name
            "/name",           // empty org, and a path
        ] {
            assert!(
                hub_dir(root, not_an_id).is_none(),
                "must not guess a cache path for {not_an_id:?}"
            );
        }
    }

    /// Build a cache entry: `files` are created under a snapshot directory,
    /// each as a real file unless its name is prefixed `dangling:`.
    /// Build a cache entry holding `files` as real files under one snapshot.
    fn entry(root: &Path, model: &str, files: &[&str]) -> PathBuf {
        let dir = hub_dir(root, model).expect("hub id");
        let snap = dir.join("snapshots").join("deadbeef");
        std::fs::create_dir_all(&snap).expect("mkdir");
        std::fs::create_dir_all(dir.join("blobs")).expect("mkdir blobs");
        for f in files {
            std::fs::write(snap.join(f), b"x").expect("write");
        }
        dir
    }

    /// Add a weight whose blob is missing — the state a partial cache cleanup
    /// leaves behind.
    ///
    /// Unix-only, and kept OUT of `entry` on purpose: expressing it as a
    /// magic filename prefix meant the Windows build had a bound name it could
    /// not use, which `#![deny(warnings)]` rejects. A separate function is
    /// gated as a whole, so there is nothing left over to be unused.
    #[cfg(unix)]
    fn add_dangling_weight(dir: &Path, name: &str) {
        let snap = dir.join("snapshots").join("deadbeef");
        std::os::unix::fs::symlink(dir.join("blobs/gone"), snap.join(name)).expect("symlink");
    }

    #[test]
    fn a_directory_that_exists_but_holds_no_weights_is_not_a_present_model() {
        let tmp = std::env::temp_dir().join(format!("atlasctl-hf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        // Exactly the shape observed on a real box: refs, a snapshot, and a
        // config.json — and not one weight file.
        let meta = entry(&tmp, "org/metadata-only", &["config.json"]);
        assert_eq!(cache_state(&meta), CacheState::MetadataOnly);
        assert!(
            meta.exists(),
            "the bug this guards: the directory IS there, which is why \
             `Path::exists` passed it through as a usable model"
        );

        let full = entry(&tmp, "org/complete", &["config.json", "model.safetensors"]);
        assert_eq!(cache_state(&full), CacheState::Weights);

        assert_eq!(
            cache_state(&hub_dir(&tmp, "org/never-fetched").unwrap()),
            CacheState::Absent
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Weights nested inside a snapshot still count.
    ///
    /// Every checkpoint in the cache this was written against is flat, so a
    /// top-level-only scan passed every test and every real model on that
    /// machine — and would have REFUSED a launch for the first Hub repo that
    /// nests them. That direction is the one an operator cannot argue with:
    /// the model is right there and the tool says it is not.
    #[test]
    fn a_weight_in_a_subdirectory_still_counts() {
        let tmp = std::env::temp_dir().join(format!("atlasctl-hf-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let dir = entry(&tmp, "org/nested", &["config.json"]);
        let inner = dir.join("snapshots/deadbeef/weights");
        std::fs::create_dir_all(&inner).expect("mkdir inner");
        std::fs::write(inner.join("model.safetensors"), b"w").expect("write");
        assert_eq!(cache_state(&dir), CacheState::Weights);

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// ...but not past the depth bound, so this stays a preflight check and
    /// cannot become a walk of whatever is mounted under the cache.
    #[test]
    fn the_search_stops_at_the_depth_bound() {
        let tmp = std::env::temp_dir().join(format!("atlasctl-hf-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let dir = entry(&tmp, "org/deep", &["config.json"]);
        let deep = dir
            .join("snapshots/deadbeef")
            .join("a")
            .join("b")
            .join("c")
            .join("d");
        std::fs::create_dir_all(&deep).expect("mkdir deep");
        std::fs::write(deep.join("model.safetensors"), b"w").expect("write");
        assert_eq!(cache_state(&dir), CacheState::MetadataOnly);

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A reclaimed blob leaves the symlink behind. Counting the link rather than
    /// its target would report weights that cannot be read.
    #[cfg(unix)]
    #[test]
    fn a_weight_whose_blob_was_reclaimed_does_not_count() {
        let tmp = std::env::temp_dir().join(format!("atlasctl-hf-dangle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let d = entry(&tmp, "org/reclaimed", &["config.json"]);
        add_dangling_weight(&d, "model.safetensors");
        assert_eq!(cache_state(&d), CacheState::MetadataOnly);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
