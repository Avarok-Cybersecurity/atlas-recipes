// SPDX-License-Identifier: AGPL-3.0-only

//! The fleet half of the browser session: what the page may ask about other
//! machines, and about running across them.
//!
//! Split from the launch half because the two answer to different things. A
//! launch verb reaches the container runtime on *this* box; a fleet verb
//! reaches another machine over the authenticated peer channel, or reaches
//! nothing at all. Keeping them apart means the peer-reaching surface stays
//! small enough to read in one sitting.

use super::{Session, err};
use atlasctl_protocol::msg::{AgentError, ServerMsg};
use atlasctl_protocol::settings::SettingValue;
use atlasctl_protocol::{RecipeId, fleet::NodeId};
use std::collections::BTreeMap;

impl Session<'_> {
    /// The fleet, local node first.
    pub(super) fn nodes(&self, id: u32) -> Vec<ServerMsg> {
        let nodes = self
            .deps
            .fleet
            .map(crate::fleet::FleetView::nodes)
            .unwrap_or_default();
        vec![ServerMsg::Nodes { id, nodes }]
    }

    /// Open a window in which one new machine may join.
    ///
    /// The addresses go out with the code because the joining machine has to
    /// dial back and cannot discover this one — it is not paired yet, and a
    /// beacon is not something to build a command on.
    pub(super) fn mint_join(&mut self, id: u32) -> Vec<ServerMsg> {
        let Some(joining) = self.deps.joining else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match joining.mint() {
            Ok(code) => vec![ServerMsg::JoinInvitation {
                id,
                code: Some(code),
                addresses: self.dialable_addresses(),
                expires_in_s: crate::joining::JOIN_TTL.as_secs(),
            }],
            // The only way minting fails is a poisoned lock, which is this
            // agent being broken rather than the request being wrong.
            Err(_) => vec![err(Some(id), AgentError::NotReady)],
        }
    }

    /// Close an outstanding window. Answering with no code is how the page
    /// learns it is shut.
    pub(super) fn revoke_join(&mut self, id: u32) -> Vec<ServerMsg> {
        if let Some(joining) = self.deps.joining {
            joining.revoke();
        }
        vec![ServerMsg::JoinInvitation {
            id,
            code: None,
            addresses: Vec::new(),
            expires_in_s: 0,
        }]
    }

    /// Where a joining machine should dial this one, best link first.
    ///
    /// Loopback and virtual links are excluded: they are reachable from here
    /// and from nowhere else, so putting one in an invitation produces a
    /// command that cannot work.
    fn dialable_addresses(&self) -> Vec<String> {
        let Some(fleet) = self.deps.fleet else {
            return Vec::new();
        };
        let mut addrs: Vec<_> = fleet
            .nodes()
            .into_iter()
            .find(|n| n.is_local)
            .map(|n| n.addresses)
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.class.usable_for_cluster())
            .collect();
        addrs.sort_by_key(|a| std::cmp::Reverse(a.class.rank()));
        addrs.into_iter().map(|a| a.addr).collect()
    }

    pub(super) fn pair(&mut self, id: u32, node: NodeId, code: &str) -> Vec<ServerMsg> {
        let Some(fleet) = self.deps.fleet else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match fleet.pair(node, code) {
            Ok(outcome) => vec![ServerMsg::PairResult {
                id,
                node,
                paired: true,
                verification: Some(outcome.verification),
                detail: String::new(),
            }],
            // A failed pairing is reported as a result rather than an error:
            // the page has a designed state for "that did not work", and the
            // reason is the useful part.
            Err(e) => vec![ServerMsg::PairResult {
                id,
                node,
                paired: false,
                verification: None,
                detail: e.to_string(),
            }],
        }
    }

    pub(super) fn unpair(&mut self, id: u32, node: NodeId) -> Vec<ServerMsg> {
        let Some(fleet) = self.deps.fleet else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match fleet.unpair(node) {
            Ok(was_pinned) => vec![ServerMsg::PairResult {
                id,
                node,
                paired: false,
                verification: None,
                detail: if was_pinned {
                    String::new()
                } else {
                    "that node was not paired".to_owned()
                },
            }],
            Err(e) => vec![err(
                Some(id),
                AgentError::InvalidMessage {
                    detail: e.to_string(),
                },
            )],
        }
    }

    pub(super) fn preview_cluster(
        &self,
        id: u32,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        let Some(cluster) = self.deps.cluster else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match cluster.preview(recipe, nodes, head, settings) {
            Ok((ranks, link_warning)) => vec![ServerMsg::ClusterPreview {
                id,
                ranks,
                link_warning,
            }],
            // NotLaunchable rather than BadSettings: the plan failing is
            // usually about the machines — one unpaired, one with no usable
            // link, the wrong count selected — not about a value being out of
            // range, and the reason says which.
            Err(reason) => vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe.clone(),
                    reason,
                },
            )],
        }
    }

    /// Ask every selected node to validate and reserve. Nothing starts.
    pub(super) fn prepare_cluster(
        &self,
        id: u32,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        let Some(cluster) = self.deps.cluster else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match cluster.prepare(recipe, nodes, head, settings) {
            Ok((epoch, ranks, may_commit)) => vec![ServerMsg::ClusterPrepared {
                id,
                epoch,
                ranks,
                may_commit,
            }],
            Err(reason) => vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe.clone(),
                    reason,
                },
            )],
        }
    }

    /// Start what every rank prepared under this epoch.
    pub(super) fn commit_cluster(&self, id: u32, epoch: &str) -> Vec<ServerMsg> {
        let Some(cluster) = self.deps.cluster else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match cluster.commit(epoch) {
            Ok(ranks) => vec![ServerMsg::ClusterStarted {
                id,
                epoch: epoch.to_owned(),
                ranks,
            }],
            // LaunchFailed rather than NotLaunchable: the plan was accepted by
            // every rank, so this is an execution failure, and by the time it
            // is reported every rank that started has been stopped again.
            Err(detail) => vec![err(Some(id), AgentError::LaunchFailed { detail })],
        }
    }

    /// Abandon a prepare, releasing every reservation.
    ///
    /// Answered with the same shape as a prepare that nobody accepted, so the
    /// page has one code path for "this launch is not going to happen".
    pub(super) fn abort_cluster(&self, id: u32, epoch: &str) -> Vec<ServerMsg> {
        let Some(cluster) = self.deps.cluster else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        cluster.abort(epoch);
        vec![ServerMsg::ClusterPrepared {
            id,
            epoch: epoch.to_owned(),
            ranks: Vec::new(),
            may_commit: false,
        }]
    }

    /// Stop every rank of the running cluster.
    pub(super) fn stop_cluster(&self, id: u32) -> Vec<ServerMsg> {
        let Some(cluster) = self.deps.cluster else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match cluster.stop_cluster() {
            Ok(ranks) => vec![ServerMsg::ClusterStopped { id, ranks }],
            Err(detail) => vec![err(Some(id), AgentError::LaunchFailed { detail })],
        }
    }
}
