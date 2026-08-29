// SPDX-License-Identifier: AGPL-3.0-only

//! What makes two nodes agree they would launch the same thing.
//!
//! Before a cluster starts, every rank compares this hash against the head's.
//! A mismatch refuses unconditionally, because two nodes running different
//! revisions of one recipe would launch two different models and call it one
//! cluster — the failure would show up as wrong output, not as an error.
//!
//! **The hash covers meaning, not bytes.** Hashing the raw YAML would make a
//! reformatted comment look like a different model, and an operator who is
//! told "these recipes differ" about a whitespace change learns to ignore the
//! warning — which is worse than not having it. So the input is exactly the
//! fields that reach the command line, in a canonical order, and nothing else:
//! a recipe's name, provenance and prose can differ between nodes without
//! consequence, because none of them change what runs.
//!
//! Fields are length-prefixed rather than concatenated, so that
//! `model="ab", container="c"` and `model="a", container="bc"` cannot collide.

use super::{Recipe, RuntimeKind};
use crate::ScalarValue;
use sha2::{Digest, Sha256};

/// Domain separator, so this hash can never be confused with another SHA-256
/// over similar material.
const DOMAIN: &[u8] = b"atlasctl.recipe.v1";

impl Recipe {
    /// The content hash every rank compares before a cluster launch.
    ///
    /// Stable across builds and machines for a given recipe revision, and
    /// sensitive to every field that changes what would be executed.
    #[must_use]
    pub fn content_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(DOMAIN);

        field(&mut h, b"recipe_version", self.recipe_version.as_bytes());
        field(&mut h, b"model", self.model.as_bytes());
        match &self.model_revision {
            Some(r) => field(&mut h, b"model_revision", r.as_bytes()),
            // Distinct from the empty string: a recipe pinning `revision: ""`
            // and one pinning nothing are not the same recipe.
            None => field(&mut h, b"model_revision", b"\x00none"),
        }
        field(&mut h, b"runtime", runtime_tag(&self.runtime).as_bytes());
        field(&mut h, b"container", self.container.as_bytes());
        field(
            &mut h,
            b"topology",
            format!(
                "{}:{}",
                self.topology.min_nodes,
                self.topology
                    .max_nodes
                    .map_or_else(|| "unbounded".to_owned(), |m| m.to_string())
            )
            .as_bytes(),
        );

        // BTreeMap iteration is ordered, so two nodes that parsed the same
        // recipe hash it identically regardless of the order keys appeared in
        // the file.
        field(&mut h, b"defaults", b"");
        for (k, v) in &self.defaults {
            field(&mut h, k.as_bytes(), scalar_tag(v).as_bytes());
        }
        field(&mut h, b"env", b"");
        for (k, v) in &self.env {
            field(&mut h, k.as_bytes(), v.as_bytes());
        }

        hex(&h.finalize())
    }
}

/// Feed one length-prefixed field, so no two different field splits can produce
/// the same byte stream.
fn field(h: &mut Sha256, key: &[u8], value: &[u8]) {
    h.update(u32::try_from(key.len()).unwrap_or(u32::MAX).to_be_bytes());
    h.update(key);
    h.update(u32::try_from(value.len()).unwrap_or(u32::MAX).to_be_bytes());
    h.update(value);
}

/// Tag a scalar with its type as well as its text.
///
/// Without the type, `1` and `"1"` would hash alike — and they do not render
/// alike on the command line.
fn scalar_tag(v: &ScalarValue) -> String {
    match v {
        ScalarValue::Bool(b) => format!("b:{b}"),
        ScalarValue::Int(i) => format!("i:{i}"),
        // Rendered the way it will be rendered onto the command line, so two
        // recipes whose floats print identically hash identically.
        ScalarValue::Float(_) => format!("f:{}", v.render()),
        ScalarValue::Str(s) => format!("s:{s}"),
    }
}

fn runtime_tag(r: &RuntimeKind) -> String {
    match r {
        RuntimeKind::Atlas => "atlas".to_owned(),
        RuntimeKind::Unsupported(name) => format!("unsupported:{name}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use crate::recipe::{Provenance, Recipe};

    fn hash_of(name: &str, yaml: &str) -> String {
        Recipe::parse(
            name,
            yaml,
            Provenance::Builtin {
                path: "fixture/r.yaml".into(),
            },
        )
        .expect("parses")
        .content_hash()
    }

    const BASE: &str = "model: org/m\ncontainer: img:tag\nruntime: atlas\n";

    #[test]
    fn the_same_recipe_hashes_the_same_way_twice() {
        assert_eq!(hash_of("r", BASE), hash_of("r", BASE));
    }

    /// The whole point of hashing meaning rather than bytes: an operator told
    /// "these recipes differ" about a comment learns to ignore the warning.
    #[test]
    fn prose_layout_and_identity_do_not_change_the_hash() {
        let base = hash_of("r", BASE);
        for (what, yaml) in [
            (
                "a comment",
                "# a note\nmodel: org/m\ncontainer: img:tag\nruntime: atlas\n",
            ),
            (
                "key order",
                "runtime: atlas\ncontainer: img:tag\nmodel: org/m\n",
            ),
            (
                "blank lines",
                "model: org/m\n\n\ncontainer: img:tag\nruntime: atlas\n",
            ),
            (
                "a description",
                "model: org/m\ncontainer: img:tag\nruntime: atlas\ndescription: hello\n",
            ),
        ] {
            assert_eq!(base, hash_of("r", yaml), "{what} must not change the hash");
        }
        // Nor does the name it was filed under, or where it came from.
        assert_eq!(base, hash_of("different-name", BASE));
    }

    #[test]
    fn every_field_that_reaches_the_command_line_changes_the_hash() {
        let base = hash_of("r", BASE);
        for (what, yaml) in [
            (
                "model",
                "model: org/other\ncontainer: img:tag\nruntime: atlas\n",
            ),
            (
                "container",
                "model: org/m\ncontainer: img:other\nruntime: atlas\n",
            ),
            (
                "runtime",
                "model: org/m\ncontainer: img:tag\nruntime: vllm-distributed\n",
            ),
            (
                "revision",
                "model: org/m\ncontainer: img:tag\nruntime: atlas\nmodel_revision: abc\n",
            ),
            (
                "recipe_version",
                "recipe_version: '1'\nmodel: org/m\ncontainer: img:tag\nruntime: atlas\n",
            ),
            (
                "a default",
                "model: org/m\ncontainer: img:tag\nruntime: atlas\ndefaults:\n  port: 8888\n",
            ),
            (
                "env",
                "model: org/m\ncontainer: img:tag\nruntime: atlas\nenv:\n  FOO: bar\n",
            ),
            (
                "topology",
                "model: org/m\ncontainer: img:tag\nruntime: atlas\nmin_nodes: 2\n",
            ),
        ] {
            assert_ne!(base, hash_of("r", yaml), "{what} must change the hash");
        }
    }

    /// `port: 8888` and `port: "8888"` render differently onto the command
    /// line, so they must not hash alike.
    #[test]
    fn a_scalars_type_is_part_of_its_identity() {
        let as_int = hash_of(
            "r",
            "model: org/m\ncontainer: img:tag\nruntime: atlas\ndefaults:\n  x: 1\n",
        );
        let as_str = hash_of(
            "r",
            "model: org/m\ncontainer: img:tag\nruntime: atlas\ndefaults:\n  x: '1'\n",
        );
        let as_bool = hash_of(
            "r",
            "model: org/m\ncontainer: img:tag\nruntime: atlas\ndefaults:\n  x: true\n",
        );
        assert_ne!(as_int, as_str);
        assert_ne!(as_int, as_bool);
        assert_ne!(as_str, as_bool);
    }

    /// Without length prefixes, moving a character across a field boundary
    /// would leave the hashed byte stream unchanged.
    #[test]
    fn moving_a_character_between_fields_changes_the_hash() {
        let a = hash_of("r", "model: ab\ncontainer: c:t\nruntime: atlas\n");
        let b = hash_of("r", "model: a\ncontainer: bc:t\nruntime: atlas\n");
        assert_ne!(a, b);
    }

    /// An absent pin and an empty pin are different recipes.
    #[test]
    fn an_absent_revision_differs_from_an_empty_one() {
        let absent = hash_of("r", BASE);
        let empty = hash_of(
            "r",
            "model: org/m\ncontainer: img:tag\nruntime: atlas\nmodel_revision: ''\n",
        );
        assert_ne!(absent, empty);
    }

    #[test]
    fn the_hash_is_a_full_sha256_in_hex() {
        let h = hash_of("r", BASE);
        assert_eq!(h.len(), 64);
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}
