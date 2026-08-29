// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! The vendored Atlas Spark recipe corpus, embedded into the binary at compile
//! time.
//!
//! Recipes are compiled in rather than fetched. That is a security property,
//! not a convenience: the tool this replaces resolved recipes from a remote git
//! registry that it also marked "trusted", which let recipe-supplied shell
//! commands run on the host. Here there is no fetch step to redirect and no
//! trust flag to set — the corpus a binary ships with is the corpus it was
//! built from.

use include_dir::{Dir, include_dir};

/// Every `recipes/**/*.yaml` in this repository, as of build time.
pub static RECIPES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/recipes");

/// One embedded recipe file: its name (the file stem) and its raw YAML bytes.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedRecipe {
    /// The recipe's name, which is its filename stem — `qwen3.6-27b-fp8`.
    pub name: &'static str,
    /// Path within the corpus, for provenance display: `qwen3.6/qwen3.6-27b-fp8.yaml`.
    pub path: &'static str,
    /// The file's contents, exactly as committed.
    pub yaml: &'static str,
}

/// Walk the embedded corpus, yielding every `.yaml` file at any depth.
///
/// Files whose contents are not valid UTF-8 are skipped rather than panicking;
/// a recipe is text by definition, so a non-UTF-8 file is not a recipe.
pub fn all() -> Vec<EmbeddedRecipe> {
    let mut out = Vec::new();
    collect(&RECIPES, &mut out);
    out.sort_by_key(|r| r.name);
    out
}

fn collect(dir: &'static Dir<'static>, out: &mut Vec<EmbeddedRecipe>) {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(d) => collect(d, out),
            include_dir::DirEntry::File(f) => {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                    continue;
                }
                let (Some(name), Some(yaml), Some(path_str)) = (
                    path.file_stem().and_then(|s| s.to_str()),
                    f.contents_utf8(),
                    path.to_str(),
                ) else {
                    continue;
                };
                out.push(EmbeddedRecipe {
                    name,
                    path: path_str,
                    yaml,
                });
            }
        }
    }
}

/// Look up one embedded recipe by name (file stem).
pub fn get(name: &str) -> Option<EmbeddedRecipe> {
    all().into_iter().find(|r| r.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_is_not_empty_and_names_are_unique() {
        let all = all();
        assert!(!all.is_empty(), "no recipes were embedded");

        let mut seen = std::collections::BTreeSet::new();
        for r in &all {
            assert!(
                seen.insert(r.name),
                "duplicate recipe name {:?} — names are file stems and must be \
                 unique across the whole corpus, because `@atlas/<name>` resolves by stem",
                r.name
            );
        }
    }

    #[test]
    fn every_embedded_file_is_reachable_by_name() {
        for r in all() {
            assert!(
                get(r.name).is_some(),
                "{} embedded but not resolvable",
                r.name
            );
        }
    }
}
