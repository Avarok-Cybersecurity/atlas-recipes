// SPDX-License-Identifier: AGPL-3.0-only

//! Planning a cluster and asking every rank what it would run.
//!
//! The head plans — which machines, which ranks, which rendezvous address — and
//! then asks. It does not render another machine's command, because it does not
//! know that machine's recipe revision, flag table or hardware. Preview and
//! execution therefore come from the same code on the same box, which is the
//! only way a preview can be trusted to be what runs.
//!
//! Rank 0 is rendered locally for exactly the same reason: the head *is* the
//! machine that would run rank 0.

use anyhow::Result;
use atlasctl_agent::cluster::plan;
use atlasctl_agent::daemon::RankRenderer;
use atlasctl_agent::fleet::{FleetView, LocalFleet};
use atlasctl_agent::identity::{Identity, PinStore};
use atlasctl_agent::session::ClusterPreviewer;
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::fleet::RankPreview;
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Plans a cluster and collects each rank's own rendering.
pub struct PeerClusterPreviewer {
    fleet: Arc<LocalFleet>,
    identity: Arc<Identity>,
    pins: PinStore,
    renderer: Arc<dyn RankRenderer>,
    peer_port: u16,
    runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for PeerClusterPreviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerClusterPreviewer")
            .finish_non_exhaustive()
    }
}

impl PeerClusterPreviewer {
    /// Build a previewer.
    #[must_use]
    pub fn new(
        fleet: Arc<LocalFleet>,
        identity: Arc<Identity>,
        pins: PinStore,
        renderer: Arc<dyn RankRenderer>,
        peer_port: u16,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            fleet,
            identity,
            pins,
            renderer,
            peer_port,
            runtime,
        }
    }
}

impl ClusterPreviewer for PeerClusterPreviewer {
    fn preview(
        &self,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<(Vec<RankPreview>, Option<String>), String> {
        let all = self.fleet.nodes();
        let selected: Vec<_> = nodes
            .iter()
            .filter_map(|id| all.iter().find(|n| n.id == *id))
            .collect();
        if selected.len() != nodes.len() {
            return Err("one of those machines is not in this fleet".to_owned());
        }

        // The recipe's own node count decides how many machines are required;
        // the page cannot widen it by selecting more.
        let required = u32::try_from(nodes.len()).unwrap_or(u32::MAX);

        let plan = plan(
            recipe.as_str(),
            "",
            required,
            &selected,
            head,
            settings,
            // The epoch matters for prepare, not preview; a preview reserves
            // nothing, so it needs no identity of its own.
            "preview".to_owned(),
        )
        .map_err(|e| e.to_string())?;

        let mut out = Vec::with_capacity(plan.ranks.len());
        for assignment in &plan.ranks {
            let node = selected
                .iter()
                .find(|n| n.id == assignment.node)
                .ok_or_else(|| "the plan named a machine that left the fleet".to_owned())?;

            let (command, _unmapped) = if node.is_local {
                // We are the machine that would run this rank.
                self.renderer
                    .render(assignment)
                    .map_err(|e| format!("{}: {e}", node.name))?
            } else {
                let addr = node
                    .preferred_address()
                    .ok_or_else(|| format!("{} has no usable network link", node.name))?;
                let sock = format!("{}:{}", addr.addr, self.peer_port)
                    .parse()
                    .map_err(|_| format!("{} has an address we cannot dial", node.name))?;
                let identity = Arc::clone(&self.identity);
                let pins = self.pins.clone();
                let assignment = assignment.clone();
                let id = node.id;
                // `block_on` alone would deadlock: this runs inside a task on
                // the very runtime it would block. `block_in_place` moves this
                // thread out of the async pool first, which is only sound on a
                // multi-threaded runtime — and that is what the agent builds.
                tokio::task::block_in_place(|| {
                    self.runtime.block_on(async move {
                        atlasctl_agent::peer::link::preview_rank(
                            &identity, pins, sock, id, assignment,
                        )
                        .await
                    })
                })
                .map_err(|e| format!("{}: {e}", node.name))?
            };

            out.push(RankPreview {
                node: assignment.node,
                name: node.name.clone(),
                rank: assignment.rank,
                master_addr: assignment.master_addr.clone(),
                command,
            });
        }

        Ok((out, plan.link_warning))
    }
}
