// SPDX-License-Identifier: AGPL-3.0-only

//! Recipe scalar values and their rendering.

use serde::{Deserialize, Serialize};

/// A YAML scalar as it appears in a recipe's `defaults:` block.
///
/// The YAML type is preserved rather than collapsed to a string, because it
/// changes what gets emitted: a bare toggle is emitted only when its value is
/// truthy, so `speculative: false` must render *nothing*, whereas
/// `disable_tool_grammar: false` is a value flag and must render the literal
/// `false`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScalarValue {
    /// `true` / `false`.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// Anything else — a dtype name, a policy, a model id.
    Str(String),
}

impl ScalarValue {
    /// Render for the command line.
    ///
    /// Floats are the subtle case. The reference implementation is Python, and
    /// `str(1.0)` is `"1.0"` there while Rust's `Display` gives `"1"`. A recipe
    /// pinning `gpu_memory_utilization: 1.0` would therefore render differently
    /// in the two implementations and silently break byte-parity with the
    /// golden corpus, so whole floats keep their `.0`.
    pub fn render(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Float(f) => {
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 {
                    format!("{f:.1}")
                } else {
                    f.to_string()
                }
            }
            Self::Str(s) => s.clone(),
        }
    }

    /// Whether a bare toggle keyed to this value should be emitted.
    ///
    /// Mirrors Python truthiness for the shapes a recipe can actually hold: a
    /// non-zero number, a non-empty string, or `true`. Note `"false"` as a
    /// *string* is truthy in Python, and we match that deliberately rather than
    /// being helpfully different.
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_floats_keep_their_decimal_point() {
        // The parity-critical case: Rust's Display would give "1", Python "1.0".
        assert_eq!(ScalarValue::Float(1.0).render(), "1.0");
        assert_eq!(ScalarValue::Float(0.0).render(), "0.0");
    }

    #[test]
    fn fractional_floats_render_like_python() {
        // Values drawn from the real recipe corpus.
        assert_eq!(ScalarValue::Float(0.88).render(), "0.88");
        assert_eq!(ScalarValue::Float(0.7).render(), "0.7");
        assert_eq!(ScalarValue::Float(0.85).render(), "0.85");
        assert_eq!(ScalarValue::Float(0.92).render(), "0.92");
    }

    #[test]
    fn ints_and_strings_and_bools_render_plainly() {
        assert_eq!(ScalarValue::Int(8192).render(), "8192");
        assert_eq!(ScalarValue::Str("fp8".into()).render(), "fp8");
        assert_eq!(ScalarValue::Bool(true).render(), "true");
        assert_eq!(ScalarValue::Bool(false).render(), "false");
    }

    #[test]
    fn truthiness_decides_whether_a_toggle_is_emitted() {
        assert!(ScalarValue::Bool(true).is_truthy());
        assert!(!ScalarValue::Bool(false).is_truthy());
        assert!(!ScalarValue::Int(0).is_truthy());
        assert!(ScalarValue::Int(1).is_truthy());
        assert!(!ScalarValue::Str(String::new()).is_truthy());
        // Python-compatible on purpose: a non-empty string is truthy even when
        // it spells "false". Diverging here would be a silent behaviour change.
        assert!(ScalarValue::Str("false".into()).is_truthy());
    }
}
