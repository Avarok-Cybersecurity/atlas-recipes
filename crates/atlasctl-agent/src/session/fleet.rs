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
    ///
    /// Wireless is INCLUDED. This used to filter on `usable_for_cluster`, which
    /// answers "can this link carry a collective?" — no, for Wi-Fi. But the
    /// joining machine only has to reach this one to pair, and a laptop on
    /// Wi-Fi is the canonical inviter: it cannot run models, so it invites a
    /// machine that can. Filtering it out left the invitation with no address
    /// and the page with an empty command to copy.
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
            .filter(|a| a.class.usable_for_control())
            .collect();
        // Still ranked, so a wired link is offered ahead of Wi-Fi when a
        // machine has both. Including wireless changes what is POSSIBLE, not
        // what is preferred.
        addrs.sort_by_key(|a| std::cmp::Reverse(a.class.rank()));
        addrs.into_iter().map(|a| a.addr).collect()
    }

    pub(super) fn pair(&mut self, id: u32, node: NodeId, code: &str) -> Vec<ServerMsg> {
        // (helper `decision` is defined at the bottom of this file)
        let Some(fleet) = self.deps.fleet else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match fleet.pair(node, code) {
            Ok(outcome) => {
                let verification = outcome.verification.clone();
                // Held, not written. Replacing any previous one: the UI shows a
                // single dialog, and a superseded exchange's words are stale.
                self.pending_pairing = Some(super::PendingPairing {
                    outcome,
                    at: std::time::Instant::now(),
                });
                vec![ServerMsg::PairResult {
                    id,
                    node,
                    exchanged: true,
                    verification: Some(verification),
                    detail: String::new(),
                }]
            }
            // A failed pairing is reported as a result rather than an error:
            // the page has a designed state for "that did not work", and the
            // reason is the useful part.
            Err(e) => vec![ServerMsg::PairResult {
                id,
                node,
                exchanged: false,
                verification: None,
                detail: e.to_string(),
            }],
        }
    }

    /// Trust the exchange this session is holding.
    pub(super) fn confirm_pairing(&mut self, id: u32, node: NodeId) -> Vec<ServerMsg> {
        // `take` FIRST, before any other precondition. Whatever the answer,
        // this exchange is spent: a decision that failed for an unrelated
        // reason must not leave the words live for a later confirm to reuse.
        let Some(pending) = self.pending_pairing.take() else {
            return vec![decision(
                id,
                node,
                false,
                "there is no exchange waiting on this connection. Pair again — the words are only meaningful for the exchange that produced them.",
            )];
        };
        if pending.outcome.node != node {
            return vec![decision(
                id,
                node,
                false,
                "that is not the machine this connection just paired with. Nothing was trusted.",
            )];
        }
        if pending.at.elapsed() > super::PENDING_PAIRING_TTL {
            return vec![decision(
                id,
                node,
                false,
                "those words are too old to act on. Pair again.",
            )];
        }
        let Some(fleet) = self.deps.fleet else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match fleet.trust(&pending.outcome) {
            Ok(()) => vec![decision(id, node, true, "")],
            // The pin did not reach disk, so this machine does NOT trust the
            // peer — even though the peer may already trust it. Saying so is
            // the only way the operator can tell that apart from success.
            Err(e) => vec![decision(
                id,
                node,
                false,
                &format!(
                    "the exchange completed but the pin could not be written: {e}. This machine does not trust {} — pair again once that is fixed.",
                    node.short()
                ),
            )],
        }
    }

    /// Discard the exchange this session is holding.
    ///
    /// What the operator is saying is "those words did not match", which is the
    /// one thing the ceremony exists to detect. Nothing was written, so there is
    /// nothing to undo — which is the whole point of the split.
    pub(super) fn reject_pairing(&mut self, id: u32, node: NodeId) -> Vec<ServerMsg> {
        let had = self.pending_pairing.take().is_some();
        vec![decision(
            id,
            node,
            false,
            if had {
                ""
            } else {
                "there was no exchange waiting; nothing was trusted."
            },
        )]
    }

    pub(super) fn unpair(&mut self, id: u32, node: NodeId) -> Vec<ServerMsg> {
        let Some(fleet) = self.deps.fleet else {
            return vec![err(Some(id), AgentError::NotReady)];
        };
        match fleet.unpair(node) {
            // `PairDecision`, not `PairResult`: unpair runs no exchange, and a
            // reply shaped like one would have to claim `exchanged: false`,
            // which is true only in the sense that nothing happened.
            Ok(was_pinned) => vec![decision(
                id,
                node,
                false,
                if was_pinned {
                    ""
                } else {
                    "that node was not paired"
                },
            )],
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

/// A trust decision, in the one shape all three verbs answer with.
fn decision(id: u32, node: NodeId, trusted: bool, detail: &str) -> ServerMsg {
    ServerMsg::PairDecision {
        id,
        node,
        trusted,
        detail: detail.to_owned(),
    }
}
