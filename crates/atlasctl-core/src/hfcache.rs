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
}
