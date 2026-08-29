// SPDX-License-Identifier: AGPL-3.0-only

//! Tell cargo that the embedded recipes are an input.
//!
//! `src/lib.rs` embeds `recipes/` with `include_dir!`, which cargo cannot see:
//! it tracks `.rs` files, not what a macro reads. So adding, editing or deleting
//! a recipe left a STALE `atlas-recipes-data` in the build cache, and every
//! consumer kept the old set.
//!
//! That is not a theoretical staleness. It means `cargo test` passes on a
//! machine that has built before and fails in CI, which builds clean -- the
//! worst direction for a check to be wrong in, because the person who added the
//! recipe sees green and the reviewer sees red. It cost exactly that on the
//! Flash-Next recipe: the golden test reported "no golden" on CI while blessing
//! it locally produced nothing at all, because the recipe was not in the crate
//! the test had linked against.

fn main() {
    // The directory itself, so a new or deleted file is noticed...
    println!("cargo:rerun-if-changed=recipes");
    // ...and every file under it, so an EDIT is noticed too: a directory's
    // mtime does not change when a file inside it is rewritten in place.
    if let Ok(walk) = std::fs::read_dir("recipes") {
        for entry in walk.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(inner) = std::fs::read_dir(&path) {
                    for f in inner.flatten() {
                        println!("cargo:rerun-if-changed={}", f.path().display());
                    }
                }
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}
