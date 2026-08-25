// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! Recipe model and deterministic recipe-to-docker translation.

pub mod flags;
pub mod recipe;
pub mod scalar;

pub use recipe::{NotLaunchable, Provenance, Recipe, RecipeError, RuntimeKind, Topology};
pub use scalar::ScalarValue;
