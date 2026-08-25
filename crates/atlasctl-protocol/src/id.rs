// SPDX-License-Identifier: AGPL-3.0-only

//! The recipe identifier, and why it is a type rather than a string.

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// Longest identifier we will accept.
const MAX_LEN: usize = 64;

/// A validated recipe name.
///
/// Parse, don't validate: the only way to obtain one is [`RecipeId::parse`],
/// and `Deserialize` routes through it, so an invalid id cannot exist anywhere
/// in the program. A webpage can name a recipe; it cannot name a path, a flag,
/// or anything a shell would find interesting, because those never become a
/// `RecipeId` in the first place.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RecipeId(String);

/// Why a candidate identifier was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecipeIdError {
    /// Empty, or longer than we allow.
    #[error("recipe id must be 1..={MAX_LEN} characters, got {0}")]
    Length(usize),

    /// Contains something outside the permitted alphabet.
    #[error("recipe id contains a character that is not [a-z0-9.-]: {0:?}")]
    Charset(char),

    /// Starts or ends with punctuation, or contains `..`.
    #[error("recipe id has malformed punctuation: {0}")]
    Punctuation(&'static str),
}

impl RecipeId {
    /// Validate a candidate identifier.
    ///
    /// The alphabet is lowercase alphanumerics, `.` and `-`, which is exactly
    /// what recipe filenames use. Everything excluded is excluded on purpose:
    /// a leading `-` would let an id be read as a flag, `..` and `/` would let
    /// it escape a directory, and uppercase would let two ids differ only by
    /// case on a case-insensitive filesystem.
    pub fn parse(s: &str) -> Result<Self, RecipeIdError> {
        if s.is_empty() || s.len() > MAX_LEN {
            return Err(RecipeIdError::Length(s.len()));
        }
        if let Some(c) = s
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '.' || *c == '-'))
        {
            return Err(RecipeIdError::Charset(c));
        }
        let first = s.chars().next().unwrap_or('-');
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(RecipeIdError::Punctuation(
                "must start with a letter or digit",
            ));
        }
        let last = s.chars().last().unwrap_or('-');
        if last == '.' || last == '-' {
            return Err(RecipeIdError::Punctuation("must not end with `.` or `-`"));
        }
        if s.contains("..") {
            return Err(RecipeIdError::Punctuation("must not contain `..`"));
        }
        Ok(Self(s.to_string()))
    }

    /// The identifier as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RecipeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RecipeId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_recipe_names_are_accepted() {
        for name in [
            "qwen3.6-27b-fp8",
            "qwen3.5-122b-a10b-nvfp4-ep2",
            "deepseek-v4-flash-nvfp4-ep2",
            "a",
        ] {
            assert!(RecipeId::parse(name).is_ok(), "{name} should be valid");
        }
    }

    #[test]
    fn path_traversal_cannot_become_an_identifier() {
        for bad in ["../etc/passwd", "..", "a/b", "a..b", "./x"] {
            assert!(RecipeId::parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn a_flag_shaped_identifier_is_rejected() {
        // Without this an id could be read as an option by whatever it reaches.
        for bad in ["-rm", "--force", "-"] {
            assert!(RecipeId::parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn shell_metacharacters_and_whitespace_are_rejected() {
        for bad in [
            "a b", "a;b", "a|b", "a$b", "a\nb", "a'b", "a\"b", "a`b", "a&b",
        ] {
            assert!(RecipeId::parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn case_and_unicode_are_rejected() {
        // Two ids differing only by case would collide on a case-insensitive
        // filesystem; homoglyphs would let two ids look identical to a human.
        assert!(RecipeId::parse("Qwen3.6").is_err());
        assert!(
            RecipeId::parse("qwen\u{0435}").is_err(),
            "cyrillic e must be rejected"
        );
    }

    #[test]
    fn length_is_bounded_at_both_ends() {
        assert!(RecipeId::parse("").is_err());
        assert!(RecipeId::parse(&"a".repeat(MAX_LEN)).is_ok());
        assert!(RecipeId::parse(&"a".repeat(MAX_LEN + 1)).is_err());
    }

    #[test]
    fn deserialization_enforces_validation() {
        // The load-bearing property: a hostile id fails at the parse boundary,
        // before any handler logic runs at all.
        assert!(serde_json::from_str::<RecipeId>(r#""qwen3.6-27b-fp8""#).is_ok());
        for bad in [r#""../../etc/passwd""#, r#""-rm""#, r#""a b""#] {
            assert!(
                serde_json::from_str::<RecipeId>(bad).is_err(),
                "{bad} must fail to parse"
            );
        }
    }

    #[test]
    fn round_trips_through_json() {
        let id = RecipeId::parse("qwen3.6-27b-fp8").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(
            json, r#""qwen3.6-27b-fp8""#,
            "must serialize as a bare string"
        );
        assert_eq!(serde_json::from_str::<RecipeId>(&json).unwrap(), id);
    }
}
