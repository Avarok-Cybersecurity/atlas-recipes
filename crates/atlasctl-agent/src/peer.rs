// SPDX-License-Identifier: AGPL-3.0-only

//! The agent-to-agent channel.
//!
//! A second listener, on a second port, speaking a second protocol, with its
//! own authentication. That separation is the guarantee: the browser channel
//! never becomes network-reachable, and it is a property you can check with
//! `ss -tlnp` rather than one that depends on branching logic staying correct
//! through a refactor.
//!
//! Nothing here executes an argv it received. Peers exchange a typed
//! [`RankAssignment`]; each agent renders its own docker command locally from
//! its own vendored recipe. The blast radius of a compromised head is therefore
//! "launch one of the recipes this machine already has, with in-range
//! parameters", not remote code execution.

pub mod cluster;
pub mod join;
pub mod link;
pub mod pair;
pub mod tls;
pub mod wire;

#[cfg(test)]
#[path = "peer/tls_tests.rs"]
mod tls_tests;

#[cfg(test)]
#[path = "peer/pair_tests.rs"]
mod pair_tests;

/// Port the peer channel listens on.
pub const DEFAULT_PEER_PORT: u16 = 34334;
