// SPDX-License-Identifier: AGPL-3.0-only

//! Serve-flag rendering: the config chain's resolved values become an argv.

mod coverage;
mod table;

pub use coverage::{EXCLUDED, excluded_reason};
pub use table::ATLAS_FLAGS;
pub use table::DEFAULT_SERVE_PORT;

use crate::scalar::ScalarValue;
use std::collections::BTreeMap;

/// How a flag consumes its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagKind {
    /// Emits `--flag <value>`.
    Value,
    /// Emits a bare `--flag` when truthy, and nothing at all when falsy.
    BoolToggle,
}

/// One serve flag: the recipe key that sets it, and how it renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagSpec {
    /// The key as written in a recipe's `defaults:` block.
    pub key: &'static str,
    /// The long flag passed to `spark serve`.
    pub flag: &'static str,
    /// Whether the flag takes a value or is a bare toggle.
    pub kind: FlagKind,
}

/// Find the spec for a recipe key, if the runtime understands it.
pub fn lookup(key: &str) -> Option<&'static FlagSpec> {
    ATLAS_FLAGS.iter().find(|s| s.key == key)
}

/// A resolved key that no flag in the table claims.
///
/// The reference implementation dropped these in silence, which is how nine
/// keys in the shipping recipe corpus — including `lm_head_dtype`, described in
/// its own recipe as a correctness pin — have never reached `spark serve`. We
/// reproduce the *emission* for byte-parity but surface every drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmappedKey {
    /// The recipe key that will not be applied.
    pub key: String,
    /// Its resolved value, for the warning message.
    pub rendered: String,
}

/// Two distinct keys renders the same flag, and both were set.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "recipe keys `{first}` and `{second}` both render `{flag}`; \
     set only one (they are aliases, and emitting the flag twice is not meaningful)"
)]
pub struct AliasConflict {
    /// The key that appears first in the table.
    pub first: String,
    /// The key that appears second in the table.
    pub second: String,
    /// The flag they collide on.
    pub flag: String,
}

/// Reject a config in which two keys would render the same flag.
pub fn validate_no_alias_conflict(
    resolved: &BTreeMap<String, ScalarValue>,
) -> Result<(), AliasConflict> {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for spec in ATLAS_FLAGS.iter() {
        if !resolved.contains_key(spec.key) {
            continue;
        }
        if let Some(prev) = seen.insert(spec.flag, spec.key) {
            return Err(AliasConflict {
                first: prev.to_string(),
                second: spec.key.to_string(),
                flag: spec.flag.to_string(),
            });
        }
    }
    Ok(())
}

/// Render the serve flags for a resolved config.
///
/// Emission order is the table's declaration order — not the map's — so output
/// is byte-stable regardless of how the config was assembled. `skip` lets a
/// caller suppress a key it will supply itself (worker ranks force `--port 0`).
///
/// Returns the argv fragment plus every key the table did not claim.
pub fn render(
    resolved: &BTreeMap<String, ScalarValue>,
    skip: &[&str],
) -> (Vec<String>, Vec<UnmappedKey>) {
    let mut argv = Vec::new();
    let mut claimed: Vec<&str> = Vec::new();

    for spec in ATLAS_FLAGS.iter() {
        if skip.contains(&spec.key) {
            claimed.push(spec.key);
            continue;
        }
        let Some(value) = resolved.get(spec.key) else {
            continue;
        };
        claimed.push(spec.key);
        match spec.kind {
            FlagKind::BoolToggle => {
                if value.is_truthy() {
                    argv.push(spec.flag.to_string());
                }
            }
            FlagKind::Value => {
                argv.push(spec.flag.to_string());
                argv.push(value.render());
            }
        }
    }

    let unmapped = resolved
        .iter()
        .filter(|(k, _)| !claimed.contains(&k.as_str()))
        .map(|(k, v)| UnmappedKey {
            key: k.clone(),
            rendered: v.render(),
        })
        .collect();

    (argv, unmapped)
}

#[cfg(test)]
mod tests;
