// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! The local agent: lets the Atlas website launch recipes on this machine.

pub mod guard;
pub mod launcher;
pub mod server;
pub mod session;
pub mod telemetry;
pub mod token;

/// Port the browser control channel listens on.
pub const DEFAULT_PORT: u16 = 34333;
