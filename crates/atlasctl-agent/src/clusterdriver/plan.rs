// SPDX-License-Identifier: AGPL-3.0-only

//! Turning a selection into a plan every rank can be asked about.
//!
//! Split from the driving half because these are the failures that cost
//! nothing: an unknown recipe, a machine that left the fleet, a machine with no
//! usable link. All of them are found here, before any rank has been asked to
//! reserve anything — so by the time the driver starts talking to machines, the
//! only failures left are theirs.

use super::{ClusterDriver, Pending, Target};
use crate::cluster::{ClusterPlan, plan};
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::fleet::{NodeDescriptor, NodeId};
use atlasctl_protocol::settings::SettingValue;
use std::collections::BTreeMap;
use std::net::SocketAddr;

impl ClusterDriver {
    /// Resolve the selection, plan, and pin every rank to a reachable address.
    ///
    /// Everything that can fail without side effects fails here: an unknown
    /// recipe, a machine that left the fleet, a machine with no usable link.
    /// By the time a rank is asked anything, the only remaining failures are
    /// that rank's own.
    pub(super) fn resolve(
        &self,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
        epoch: String,
    ) -> Result<Pending, String> {
        let all = self.fleet.nodes();
        let selected: Vec<NodeDescriptor> = nodes
            .iter()
            .filter_map(|id| all.iter().find(|n| n.id == *id).cloned())
            .collect();
        if selected.len() != nodes.len() {
            return Err("one of those machines is not in this fleet".to_owned());
        }

        // The head states the revision it intends, and every rank compares it
        // against its own. Sending nothing would make the comparison vacuous,
        // so an unknown recipe fails here rather than launching unchecked.
        let hash = self
            .rank
            .content_hash(recipe.as_str())
            .map_err(|e| format!("{e:#}"))?;

        // The recipe's own node count decides how many machines are required;
        // the page cannot widen it by selecting more.
        let required = u32::try_from(nodes.len()).unwrap_or(u32::MAX);
        let refs: Vec<&NodeDescriptor> = selected.iter().collect();
        let plan: ClusterPlan = plan(
            recipe.as_str(),
            &hash,
            required,
            &refs,
            head,
            settings,
            epoch.clone(),
        )
        .map_err(|e| e.to_string())?;

        let mut targets = Vec::with_capacity(plan.ranks.len());
        for assignment in plan.ranks {
            let node = selected
                .iter()
                .find(|n| n.id == assignment.node)
                .ok_or_else(|| "the plan named a machine that left the fleet".to_owned())?;
            targets.push(Target {
                addr: self.address_of(node)?,
                name: node.name.clone(),
                assignment,
            });
        }

        Ok(Pending {
            port: self.serve_port(recipe, &targets),
            epoch,
            targets,
        })
    }

    /// The link warning for a selection, computed the same way the plan does.
    pub(super) fn link_warning(
        &self,
        recipe: &RecipeId,
        nodes: &[NodeId],
        head: NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Option<String> {
        let all = self.fleet.nodes();
        let selected: Vec<NodeDescriptor> = nodes
            .iter()
            .filter_map(|id| all.iter().find(|n| n.id == *id).cloned())
            .collect();
        let refs: Vec<&NodeDescriptor> = selected.iter().collect();
        let hash = self.rank.content_hash(recipe.as_str()).ok()?;
        plan(
            recipe.as_str(),
            &hash,
            u32::try_from(nodes.len()).unwrap_or(u32::MAX),
            &refs,
            head,
            settings,
            "preview".to_owned(),
        )
        .ok()?
        .link_warning
    }

    /// Where to reach a rank, or `None` when it is this machine.
    pub(super) fn address_of(&self, node: &NodeDescriptor) -> Result<Option<SocketAddr>, String> {
        if node.is_local {
            return Ok(None);
        }
        // The peer's best address *from here*, not its best address outright.
        // A Spark answers on several point-to-point links and this machine is
        // attached to only some of them; picking by class alone dials one that
        // goes nowhere and times out.
        let local = self.fleet.nodes();
        let mine = local
            .iter()
            .find(|n| n.is_local)
            .map(|n| n.addresses.clone())
            .unwrap_or_default();
        let addr = crate::rendezvous::best_reachable(&node.addresses, &mine)
            .ok_or_else(|| format!("{} has no usable network link", node.name))?;
        format!("{}:{}", addr.addr, self.peer_port)
            .parse()
            .map(Some)
            .map_err(|_| format!("{} has an address we cannot dial", node.name))
    }
}

impl ClusterDriver {
    /// The port rank 0 will serve on, for the endpoint shown to the operator.
    ///
    /// Three layers, in the order that decides the answer:
    ///
    /// 1. What the operator set. `assignment.settings` holds **overrides only**
    ///    — a sparse diff against the recipe, not the effective settings.
    /// 2. What the recipe pins, asked of this machine's own copy.
    /// 3. The serving runtime's default.
    ///
    /// Layer 2 was missing, and its absence was invisible in exactly the case
    /// that matters: a recipe pinning `port: 8888` and an operator who did not
    /// override it produced an empty settings map, so the endpoint told them
    /// `:8000` while the model served on `:8888`. Reading a sparse diff as if
    /// it were the whole truth is the same class of bug in either direction.
    fn serve_port(&self, recipe: &RecipeId, targets: &[Target]) -> u16 {
        if let Some(SettingValue::Int(p)) = targets
            .iter()
            .find(|t| t.assignment.rank == 0)
            .and_then(|t| t.assignment.settings.get("port"))
            && let Ok(p) = u16::try_from(*p)
        {
            return p;
        }
        // A recipe this machine cannot resolve is not worth failing a plan
        // over here — the content hash above already refused that case, and
        // this value only decorates a URL.
        self.rank
            .recipe_port(recipe.as_str())
            .ok()
            .flatten()
            .unwrap_or(DEFAULT_SERVE_PORT)
    }
}

/// The port used when neither the operator nor the recipe names one.
///
/// Not a silent fallback: it is the serving runtime's own default, it is only
/// reached once both layers above have been asked, and it only ever decorates
/// a URL shown to a human — nothing is launched from it.
const DEFAULT_SERVE_PORT: u16 = 8000;
