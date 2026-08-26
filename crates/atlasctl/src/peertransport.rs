// SPDX-License-Identifier: AGPL-3.0-only

//! Reaching another machine over the authenticated peer channel.
//!
//! The only part of cluster launch that touches a socket. Everything that
//! decides *what* to ask and what to undo lives in
//! [`atlasctl_agent::clusterdriver`], which is why this file has no branching
//! in it worth testing and that one is covered thoroughly.

use anyhow::Result;
use atlasctl_agent::cluster::{PrepareReply, RankAssignment};
use atlasctl_agent::identity::{Identity, PinStore};
use atlasctl_agent::peer::cluster;
use atlasctl_agent::transport::RankTransport;
use atlasctl_protocol::fleet::NodeId;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

/// Dials pinned peers on the agent's own runtime.
pub struct PeerTransport {
    identity: Arc<Identity>,
    pins: PinStore,
    runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for PeerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerTransport").finish_non_exhaustive()
    }
}

impl PeerTransport {
    /// Build a transport.
    #[must_use]
    pub fn new(identity: Arc<Identity>, pins: PinStore, runtime: tokio::runtime::Handle) -> Self {
        Self {
            identity,
            pins,
            runtime,
        }
    }

    /// Run one peer call to completion.
    ///
    /// `block_on` alone would deadlock: this runs inside a task on the very
    /// runtime it would block. `block_in_place` moves this thread out of the
    /// async pool first, which is only sound on a multi-threaded runtime — and
    /// that is what the agent builds.
    fn blocking<F: Future>(&self, fut: F) -> F::Output {
        tokio::task::block_in_place(|| self.runtime.block_on(fut))
    }
}

impl RankTransport for PeerTransport {
    fn preview(
        &self,
        node: NodeId,
        addr: SocketAddr,
        assignment: &RankAssignment,
    ) -> Result<(String, Vec<String>)> {
        self.blocking(cluster::preview_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            assignment.clone(),
        ))
    }

    fn prepare(
        &self,
        node: NodeId,
        addr: SocketAddr,
        epoch: &str,
        assignment: &RankAssignment,
    ) -> PrepareReply {
        self.blocking(cluster::prepare_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            epoch,
            assignment.clone(),
        ))
        // A machine that could not be reached has not agreed to anything. Said
        // as a refusal rather than an error so the driver still releases the
        // reservations the ranks before it are holding.
        .unwrap_or_else(|e| PrepareReply::Refused {
            reason: format!("could not be reached: {e:#}"),
        })
    }

    fn commit(&self, node: NodeId, addr: SocketAddr, epoch: &str) -> Result<String> {
        self.blocking(cluster::commit_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            epoch,
        ))
    }

    fn abort(&self, node: NodeId, addr: SocketAddr, epoch: &str) {
        let _ = self.blocking(cluster::abort_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            epoch,
        ));
    }

    fn alive(&self, node: NodeId, addr: SocketAddr, container: &str) -> Result<bool> {
        self.blocking(cluster::rank_alive(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            container,
        ))
    }

    fn stop(&self, node: NodeId, addr: SocketAddr, container: &str) {
        let _ = self.blocking(cluster::stop_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            container,
        ));
    }
}
