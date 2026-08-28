// SPDX-License-Identifier: AGPL-3.0-only

//! Where recipes come from, and how a name resolves to one.

mod remote;

pub use remote::{RemoteRegistry, RemoteStore, git_clone_argv, git_update_argv};

use crate::recipe::{Provenance, Recipe, RecipeError};

/// The name of the built-in registry.
pub const BUILTIN: &str = "atlas";

/// Names a remote registry may not claim.
///
/// This is a local rule with no notion of "who owns a name" — the reference
/// implementation gated registry names on the GitHub organisation of their URL,
/// which is precisely the mechanism that let an upstream reserve `atlas` to an
/// org we do not control. Here the built-in name is simply not available, and
/// there is nothing an outsider can assert to change that.
pub const RESERVED: [&str; 2] = [BUILTIN, "builtin"];

/// A parsed recipe reference as the user wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeRef {
    /// `@registry/name` — explicit about where it should come from.
    Scoped {
        /// The registry name.
        registry: String,
        /// The recipe name.
        name: String,
    },
    /// A bare name, resolved built-in first.
    Bare(String),
    /// A path to a recipe file.
    Path(String),
}

impl RecipeRef {
    /// Parse a reference.
    ///
    /// A value containing a path separator, or ending in a YAML extension, is a
    /// path; `@reg/name` is scoped; anything else is a bare name.
    pub fn parse(input: &str) -> Self {
        if let Some(rest) = input.strip_prefix('@')
            && let Some((registry, name)) = rest.split_once('/')
            && !registry.is_empty()
            && !name.is_empty()
        {
            return Self::Scoped {
                registry: registry.to_string(),
                name: name.to_string(),
            };
        }
        if input.contains('/') || input.ends_with(".yaml") || input.ends_with(".yml") {
            return Self::Path(input.to_string());
        }
        Self::Bare(input.to_string())
    }
}

/// Why a reference could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// No recipe of that name in that registry.
    #[error("no recipe named `{name}` in registry `{registry}`")]
    NotFound {
        /// The recipe name.
        name: String,
        /// The registry searched.
        registry: String,
    },

    /// No recipe of that name anywhere.
    #[error("no recipe named `{name}`{}", suggestion(.close))]
    NotFoundAnywhere {
        /// The recipe name.
        name: String,
        /// Similar names, if any.
        close: Vec<String>,
    },

    /// The name exists in more than one remote registry.
    #[error(
        "`{name}` is ambiguous — it exists in {}. Qualify it, e.g. `@{first}/{name}`.",
        .registries.join(" and "),
        first = .registries.first().map(String::as_str).unwrap_or("registry")
    )]
    Ambiguous {
        /// The recipe name.
        name: String,
        /// The registries that carry it.
        registries: Vec<String>,
    },

    /// The recipe was found but could not be loaded.
    #[error(transparent)]
    Recipe(#[from] RecipeError),
}

fn suggestion(close: &[String]) -> String {
    if close.is_empty() {
        String::new()
    } else {
        format!(". Did you mean {}?", close.join(", "))
    }
}

/// A short summary for listings, without loading the whole recipe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeEntry {
    /// The recipe name.
    pub name: String,
    /// Which registry it came from.
    pub registry: String,
}

/// The set of registries a resolution searches.
pub struct RegistrySet {
    remotes: Vec<RemoteRegistry>,
}

impl RegistrySet {
    /// Just the built-in corpus — the default, and the only one that needs no
    /// network access at any point.
    pub fn builtin_only() -> Self {
        Self {
            remotes: Vec::new(),
        }
    }

    /// The built-in corpus plus explicitly-added remotes.
    pub fn with_remotes(remotes: Vec<RemoteRegistry>) -> Self {
        Self { remotes }
    }

    /// The configured remotes.
    pub fn remotes(&self) -> &[RemoteRegistry] {
        &self.remotes
    }

    /// Every recipe available, built-in first, then remotes in order.
    pub fn list(&self) -> Vec<RecipeEntry> {
        let mut out: Vec<RecipeEntry> = atlas_recipes_data::all()
            .into_iter()
            .map(|e| RecipeEntry {
                name: e.name.to_string(),
                registry: BUILTIN.to_string(),
            })
            .collect();
        for r in &self.remotes {
            for name in r.recipe_names() {
                out.push(RecipeEntry {
                    name,
                    registry: r.name.clone(),
                });
            }
        }
        out
    }

    /// Resolve a reference to a loaded recipe.
    pub fn resolve(&self, r#ref: &RecipeRef) -> Result<Recipe, ResolveError> {
        match r#ref {
            RecipeRef::Scoped { registry, name } if registry == BUILTIN => self
                .load_builtin(name)
                .ok_or_else(|| ResolveError::NotFound {
                    name: name.clone(),
                    registry: registry.clone(),
                })?,
            RecipeRef::Scoped { registry, name } => {
                let reg = self
                    .remotes
                    .iter()
                    .find(|r| &r.name == registry)
                    .ok_or_else(|| ResolveError::NotFound {
                        name: name.clone(),
                        registry: registry.clone(),
                    })?;
                reg.load(name).ok_or_else(|| ResolveError::NotFound {
                    name: name.clone(),
                    registry: registry.clone(),
                })?
            }
            RecipeRef::Bare(name) => return self.resolve_bare(name),
            RecipeRef::Path(_) => {
                return Err(ResolveError::NotFoundAnywhere {
                    name: r#ref_display(r#ref),
                    close: Vec::new(),
                });
            }
        }
        .map_err(ResolveError::from)
    }

    /// Bare names resolve built-in first, so no remote can shadow a shipped
    /// recipe by choosing its name.
    fn resolve_bare(&self, name: &str) -> Result<Recipe, ResolveError> {
        if let Some(found) = self.load_builtin(name) {
            return found.map_err(ResolveError::from);
        }
        let carriers: Vec<&RemoteRegistry> = self
            .remotes
            .iter()
            .filter(|r| r.recipe_names().iter().any(|n| n == name))
            .collect();
        match carriers.as_slice() {
            [] => Err(ResolveError::NotFoundAnywhere {
                name: name.to_string(),
                close: self.close_names(name),
            }),
            [one] => one
                .load(name)
                .ok_or_else(|| ResolveError::NotFound {
                    name: name.to_string(),
                    registry: one.name.clone(),
                })?
                .map_err(ResolveError::from),
            many => Err(ResolveError::Ambiguous {
                name: name.to_string(),
                registries: many.iter().map(|r| r.name.clone()).collect(),
            }),
        }
    }

    fn load_builtin(&self, name: &str) -> Option<Result<Recipe, RecipeError>> {
        atlas_recipes_data::get(name).map(|e| {
            Recipe::parse(
                e.name,
                e.yaml,
                Provenance::Builtin {
                    path: e.path.to_string(),
                },
            )
        })
    }

    /// The names nearest `name`, to turn a typo into a useful message.
    ///
    /// Ranked by edit distance rather than by a shared prefix. The prefix rule
    /// this replaces took the first six characters and then sorted the matches
    /// ALPHABETICALLY before truncating to three, which fails in the two ways
    /// a catalogue like this one guarantees:
    ///
    /// * `qwen3.8-27b-nvfp4-unsloh` shares `qwen3.` with every qwen3.x recipe,
    ///   so the three printed were all `qwen3.5-*` while the recipe ONE
    ///   character away never appeared. Three confident wrong answers is worse
    ///   than none.
    /// * `gemma4-31b-nvfp4` — one missing hyphen — shares no six-character
    ///   prefix with `gemma-4-31b-nvfp4`, so it got no suggestion at all.
    fn close_names(&self, name: &str) -> Vec<String> {
        let typed = name.chars().count();
        let mut scored: Vec<(usize, String)> = self
            .list()
            .into_iter()
            .map(|e| (edit_distance(name, &e.name), e.name))
            .filter(|(d, n)| *d <= max_edits(typed.max(n.chars().count())))
            .collect();
        // Distance first, then the name, so the list is stable run to run.
        scored.sort_by(|x, y| x.0.cmp(&y.0).then_with(|| x.1.cmp(&y.1)));
        scored.into_iter().take(3).map(|(_, n)| n).collect()
    }
}

/// How far a name may be from what was typed and still be worth printing.
///
/// A quarter of the longer of the two names: floored at 2 so a short name
/// still catches a near miss, and capped at 5 so an unrelated string prints
/// nothing rather than three wrong guesses.
const fn max_edits(len: usize) -> usize {
    match len / 4 {
        0 | 1 => 2,
        n if n > 5 => 5,
        n => n,
    }
}

/// Levenshtein distance over chars, two rows.
///
/// Recipe names are short and the catalogue is small, so the allocation per
/// call is not worth avoiding.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur: Vec<usize> = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn r#ref_display(r: &RecipeRef) -> String {
    match r {
        RecipeRef::Scoped { registry, name } => format!("@{registry}/{name}"),
        RecipeRef::Bare(n) | RecipeRef::Path(n) => n.clone(),
    }
}

#[cfg(test)]
mod tests;
