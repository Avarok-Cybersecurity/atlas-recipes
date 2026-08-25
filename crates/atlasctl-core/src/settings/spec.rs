// SPDX-License-Identifier: AGPL-3.0-only

//! The shape of a disposition: whether a client may set a flag, and how.

use atlasctl_protocol::settings::{Bound, Group};

/// Whether a remote client may set a flag, and how.
pub enum Disposition {
    /// Settable, within the stated bound.
    Expose(Spec),
    /// Never settable by a client, with the reason recorded.
    Deny(&'static str),
}

/// A settable flag's description, before it becomes wire data.
pub struct Spec {
    /// What values it accepts.
    pub bound: BoundSpec,
    /// Human label.
    pub label: &'static str,
    /// One sentence on what it does.
    pub help: &'static str,
    /// Unit for display.
    pub unit: Option<&'static str>,
    /// Where it belongs in the UI.
    pub group: Group,
    /// Hide behind a disclosure.
    pub advanced: bool,
}

/// A bound in static form, so the table needs no allocation.
pub enum BoundSpec {
    /// Whole number in an inclusive range.
    Int(i64, i64),
    /// Fractional number in an inclusive range.
    Float(f64, f64),
    /// One of a fixed set.
    Enum(&'static [&'static str]),
    /// Bare toggle.
    Toggle,
    /// Explicit boolean value.
    BoolValue,
    /// `auto` or a number.
    IntOrAuto(i64, i64),
}

impl BoundSpec {
    /// Convert to the wire form.
    pub fn to_bound(&self) -> Bound {
        match self {
            Self::Int(a, b) => Bound::Int { min: *a, max: *b },
            Self::Float(a, b) => Bound::Float { min: *a, max: *b },
            Self::Enum(v) => Bound::Enum {
                variants: v.iter().map(|s| (*s).to_string()).collect(),
            },
            Self::Toggle => Bound::Toggle,
            Self::BoolValue => Bound::BoolValue,
            Self::IntOrAuto(a, b) => Bound::IntOrAuto { min: *a, max: *b },
        }
    }
}
