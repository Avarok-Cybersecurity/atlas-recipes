// SPDX-License-Identifier: AGPL-3.0-only

//! The closed, typed schema of what a client may set on a launch.
//!
//! A webpage asking to start a model is untrusted input. Rather than accepting
//! key/value pairs and hoping, a client may set only keys this schema names,
//! to values inside the bounds it states. There is deliberately **no free-string
//! kind**: every settable value is a bounded number, a member of a closed
//! enumeration, a boolean, or the literal `"auto"`. A `--`-prefixed string
//! cannot survive validation, because no variant can hold one.

use serde::{Deserialize, Serialize};

/// A value a client sent for a setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    /// `true` / `false`.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A number with a fraction.
    Float(f64),
    /// Text — only ever accepted against an `Enum` or `IntOrAuto` bound.
    Str(String),
}

/// What values a setting accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bound {
    /// A whole number within an inclusive range.
    Int {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// A fractional number within an inclusive range.
    Float {
        /// Smallest accepted value.
        min: f64,
        /// Largest accepted value.
        max: f64,
    },
    /// One of a fixed set of strings.
    Enum {
        /// Every accepted value.
        variants: Vec<String>,
    },
    /// A bare toggle.
    Toggle,
    /// A boolean rendered as an explicit value rather than a bare flag.
    ///
    /// Distinct from `Toggle` because absent and false differ: the flag
    /// overrides a value the model's own configuration sets, so "unset" and
    /// "explicitly false" are not the same statement.
    BoolValue,
    /// The literal `auto`, or a whole number in range.
    IntOrAuto {
        /// Smallest accepted number.
        min: i64,
        /// Largest accepted number.
        max: i64,
    },
}

/// Which part of the UI a setting belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// Address and identity of the server.
    Server,
    /// Batching and scheduling.
    Performance,
    /// The memory and KV-cache budget.
    MemoryKv,
    /// Speculative decoding.
    Speculative,
    /// Tool calling and chat behaviour.
    ToolsChat,
    /// Topology, which the recipe usually fixes.
    Topology,
}

/// One settable knob, as described to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingSpec {
    /// The recipe key this sets. Also the foreign key into the flag table.
    pub key: String,
    /// What values it accepts.
    pub bound: Bound,
    /// Human label.
    pub label: String,
    /// One sentence on what it does.
    pub help: String,
    /// Unit for display, when it has one.
    pub unit: Option<String>,
    /// Where it belongs in the UI.
    pub group: Group,
    /// Whether to hide it behind a disclosure.
    pub advanced: bool,
    /// Set by the recipe's topology and not editable.
    pub locked: bool,
}

/// Why a submitted value was rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum SettingError {
    /// No setting by that name.
    #[error("`{key}` is not a setting this agent understands")]
    UnknownKey {
        /// The offending key.
        key: String,
    },

    /// The setting exists but clients may not set it.
    #[error("`{key}` cannot be set remotely: {reason}")]
    Denied {
        /// The offending key.
        key: String,
        /// Why it is refused.
        reason: String,
    },

    /// Out of range.
    #[error("`{key}` must be between {min} and {max}")]
    OutOfRange {
        /// The offending key.
        key: String,
        /// Lower bound, rendered.
        min: String,
        /// Upper bound, rendered.
        max: String,
    },

    /// Not one of the permitted values.
    #[error("`{key}` must be one of: {}", .allowed.join(", "))]
    NotAVariant {
        /// The offending key.
        key: String,
        /// The permitted values.
        allowed: Vec<String>,
    },

    /// Wrong shape entirely.
    #[error("`{key}` expected {expected}")]
    WrongType {
        /// The offending key.
        key: String,
        /// What was expected.
        expected: String,
    },
}

impl Bound {
    /// Check a value against this bound, returning it normalized.
    ///
    /// Numbers are re-rendered from the parsed value rather than echoed, so the
    /// bytes a client sent never reach a command line; enum matches return the
    /// stored variant rather than the submitted string, for the same reason.
    pub fn check(&self, key: &str, value: &SettingValue) -> Result<SettingValue, SettingError> {
        match (self, value) {
            (Self::Int { min, max }, SettingValue::Int(n)) => {
                if n < min || n > max {
                    return Err(SettingError::OutOfRange {
                        key: key.into(),
                        min: min.to_string(),
                        max: max.to_string(),
                    });
                }
                Ok(SettingValue::Int(*n))
            }
            (Self::Float { min, max }, v) => {
                let n = match v {
                    SettingValue::Float(f) => *f,
                    // A whole number is a legitimate spelling of a float.
                    SettingValue::Int(i) => *i as f64,
                    _ => {
                        return Err(SettingError::WrongType {
                            key: key.into(),
                            expected: "a number".into(),
                        });
                    }
                };
                if !n.is_finite() || n < *min || n > *max {
                    return Err(SettingError::OutOfRange {
                        key: key.into(),
                        min: min.to_string(),
                        max: max.to_string(),
                    });
                }
                Ok(SettingValue::Float(n))
            }
            (Self::Enum { variants }, SettingValue::Str(s)) => variants
                .iter()
                .find(|v| *v == s)
                .map(|v| SettingValue::Str(v.clone()))
                .ok_or_else(|| SettingError::NotAVariant {
                    key: key.into(),
                    allowed: variants.clone(),
                }),
            (Self::Toggle | Self::BoolValue, SettingValue::Bool(b)) => Ok(SettingValue::Bool(*b)),
            (Self::IntOrAuto { min, max }, v) => match v {
                SettingValue::Str(s) if s == "auto" => Ok(SettingValue::Str("auto".into())),
                SettingValue::Int(n) if n >= min && n <= max => Ok(SettingValue::Int(*n)),
                SettingValue::Int(_) => Err(SettingError::OutOfRange {
                    key: key.into(),
                    min: min.to_string(),
                    max: max.to_string(),
                }),
                _ => Err(SettingError::WrongType {
                    key: key.into(),
                    expected: "`auto` or a whole number".into(),
                }),
            },
            (Self::Int { .. }, _) => Err(SettingError::WrongType {
                key: key.into(),
                expected: "a whole number".into(),
            }),
            (Self::Enum { variants }, _) => Err(SettingError::NotAVariant {
                key: key.into(),
                allowed: variants.clone(),
            }),
            (Self::Toggle | Self::BoolValue, _) => Err(SettingError::WrongType {
                key: key.into(),
                expected: "true or false".into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests;
