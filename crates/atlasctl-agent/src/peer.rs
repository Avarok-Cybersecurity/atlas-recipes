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

pub mod bindfail;
pub mod cluster;
pub mod control;
pub mod join;
pub mod link;
pub mod pair;
pub mod reach;
pub mod tls;
pub mod wire;

#[cfg(test)]
#[path = "peer/tls_tests.rs"]
mod tls_tests;

#[cfg(test)]
#[path = "peer/link_tests.rs"]
mod link_tests;

#[cfg(test)]
#[path = "peer/wire_tests.rs"]
mod wire_tests;

#[cfg(test)]
#[path = "peer/pair_tests.rs"]
mod pair_tests;

/// Port the peer channel listens on.
pub const DEFAULT_PEER_PORT: u16 = 34334;

/// Whether this process's peer listener has ever come up, and on which port.
///
/// `0` means "not yet". Written once the listener binds, read by
/// `atlasctl agent status` through the agent's own status reply, so an operator
/// can see the one thing that otherwise only shows up as a "Connection refused"
/// on a DIFFERENT machine: this agent is running, but cannot accept peers.
static LISTENING_ON: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// Record that the peer listener is accepting on `port`.
pub fn mark_listener_up(port: u16) {
    LISTENING_ON.store(port, std::sync::atomic::Ordering::Relaxed);
}

/// The port the peer listener is accepting on, or `None` if it is not up.
///
/// Not a probe: a probe from inside this process would also succeed against
/// somebody ELSE's listener on the same port, which is precisely the case this
/// is meant to distinguish.
#[must_use]
pub fn listening_on() -> Option<u16> {
    match LISTENING_ON.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        p => Some(p),
    }
}
