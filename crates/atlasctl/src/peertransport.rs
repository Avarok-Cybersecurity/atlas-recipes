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
use atlasctl_agent::peer::link::SelfIntro;
use atlasctl_agent::transport::RankTransport;
use atlasctl_protocol::fleet::NodeId;
use std::net::SocketAddr;
use std::sync::Arc;

/// Dials pinned peers on the agent's own runtime.
pub struct PeerTransport {
    identity: Arc<Identity>,
    pins: PinStore,
    /// How this agent introduces itself to a rank. Built once, from the same
    /// launchability the rest of the agent reports, so a control-only head
    /// cannot describe itself as able to run a model.
    intro: SelfIntro,
}

impl std::fmt::Debug for PeerTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerTransport").finish_non_exhaustive()
    }
}

impl PeerTransport {
    /// Build a transport.
    #[must_use]
    pub fn new(identity: Arc<Identity>, pins: PinStore, intro: SelfIntro) -> Self {
        Self {
            identity,
            pins,
            intro,
        }
    }
}

impl RankTransport for PeerTransport {
    fn preview<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        assignment: &'a RankAssignment,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(String, Vec<String>)>> + Send + 'a>,
    > {
        Box::pin(cluster::preview_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            &self.intro,
            assignment.clone(),
        ))
    }

    fn prepare<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        epoch: &'a str,
        assignment: &'a RankAssignment,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = PrepareReply> + Send + 'a>> {
        Box::pin(async move {
            cluster::prepare_rank(
                &self.identity,
                self.pins.clone(),
                addr,
                node,
                &self.intro,
                epoch,
                assignment.clone(),
            )
            .await
            // A machine that could not be reached has not agreed to anything.
            // Said as a refusal rather than an error so the driver still
            // releases the reservations the ranks before it are holding.
            .unwrap_or_else(|e| PrepareReply::Refused {
                reason: format!("could not be reached: {e:#}"),
            })
        })
    }

    fn commit<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        epoch: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(cluster::commit_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            &self.intro,
            epoch,
        ))
    }

    fn abort<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        epoch: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = cluster::abort_rank(
                &self.identity,
                self.pins.clone(),
                addr,
                node,
                &self.intro,
                epoch,
            )
            .await;
        })
    }

    fn alive<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        container: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(cluster::rank_alive(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            &self.intro,
            container,
        ))
    }

    fn stop<'a>(
        &'a self,
        node: NodeId,
        addr: SocketAddr,
        container: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(cluster::stop_rank(
            &self.identity,
            self.pins.clone(),
            addr,
            node,
            &self.intro,
            container,
        ))
    }
}
