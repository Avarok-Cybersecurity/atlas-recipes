// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! The local agent: lets the Atlas website launch recipes on this machine.

pub mod cluster;
pub mod clusterdriver;
pub mod daemon;
pub mod discovery;
pub mod fabric;
pub mod fleet;
pub mod guard;
pub mod identity;
pub mod launcher;
pub mod launchstats;
pub mod pairing;
pub mod peer;
pub mod rank;
pub mod server;
pub mod session;
pub mod telemetry;
pub mod token;
pub mod transport;

/// Port the browser control channel listens on.
pub const DEFAULT_PORT: u16 = 34333;
