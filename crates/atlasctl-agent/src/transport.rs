// SPDX-License-Identifier: AGPL-3.0-only

//! Reaching another machine, behind a trait.
//!
//! The cluster state machine decides *what* to ask each rank and, more
//! importantly, what to undo when a rank says no. That logic is where a partial
//! cluster comes from, so it is the part that has to be testable — and it
//! cannot be, if asking a rank means opening a socket.
//!
//! So the driver takes one of these instead. Production passes an
//! implementation that dials the authenticated peer channel; tests pass one
//! that records calls and answers from a table, and the rollback paths can then
//! be driven through every ordering that matters without two machines, two
//! TLS stacks and a network.

use crate::cluster::{PrepareReply, RankAssignment};
use anyhow::Result;
use atlasctl_protocol::fleet::NodeId;
use std::net::SocketAddr;

/// Asking a *remote* rank to do something.
///
/// Mirrors [`crate::rank::RankService`] verb for verb, because the driver must
/// treat the head no differently from a worker. Where the two diverge is where
/// a bug hides: a head that skipped its own prepare would commit a rank nobody
/// validated.
pub trait RankTransport: Send + Sync {
    /// What this rank would run.
    ///
    /// # Errors
    /// If the peer cannot be reached or refuses.
    fn preview(
        &self,
        node: NodeId,
        addr: SocketAddr,
        assignment: &RankAssignment,
    ) -> Result<(String, Vec<String>)>;

    /// Ask this rank to validate and reserve.
    ///
    /// Returns a refusal rather than an error when the machine cannot be
    /// reached: a machine that did not answer has not agreed to anything, and
    /// the ranks that already accepted still hold reservations the driver must
    /// release.
    fn prepare(
        &self,
        node: NodeId,
        addr: SocketAddr,
        epoch: &str,
        assignment: &RankAssignment,
    ) -> PrepareReply;

    /// Start what this rank prepared.
    ///
    /// # Errors
    /// If the peer cannot be reached, holds no such reservation, or its
    /// container runtime refuses.
    fn commit(&self, node: NodeId, addr: SocketAddr, epoch: &str) -> Result<String>;

    /// Release this rank's reservation. Failures are the driver's to ignore.
    fn abort(&self, node: NodeId, addr: SocketAddr, epoch: &str);

    /// Whether a container this rank started is still running.
    ///
    /// A peer that cannot be asked is treated as dead by the caller: a rank
    /// whose liveness is unknown cannot be counted as part of a whole cluster.
    ///
    /// # Errors
    /// If the peer cannot be reached.
    fn alive(&self, node: NodeId, addr: SocketAddr, container: &str) -> Result<bool>;

    /// Stop a container this rank started. Failures are the driver's to ignore.
    fn stop(&self, node: NodeId, addr: SocketAddr, container: &str);
}
