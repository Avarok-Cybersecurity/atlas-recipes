// SPDX-License-Identifier: AGPL-3.0-only

//! Planning and committing a launch that spans several machines.
//!
//! Two properties are load-bearing, and both are enforced by the types rather
//! than by care.
//!
//! **No agent ever runs an argv it was handed.** A [`RankAssignment`] carries a
//! recipe id, a rank, a world size, a rendezvous address and bounded scalar
//! settings — and nothing else. Each worker renders its own docker command
//! locally, from its own vendored copy of the recipe. The blast radius of a
//! compromised head is therefore "start one of the recipes this machine already
//! ships, with in-range parameters", which is annoying rather than fatal.
//!
//! **There is never a partial cluster.** Prepare asks every rank to validate
//! and reserve; a single refusal aborts every rank that accepted. Commit only
//! runs once every rank is prepared under the same epoch, so a stale prepare
//! cannot be committed against a plan that has since changed.
//!
//! The planning half is pure, so the awkward cases — a head with no usable
//! link, a fleet where only one node can launch, ranks that disagree about the
//! recipe — are unit-testable with no network and no hardware.

use atlasctl_protocol::fleet::{LinkClass, NodeDescriptor, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// Port ranks rendezvous on.
pub const DEFAULT_MASTER_PORT: u16 = 29500;

/// What one node is told to do.
///
/// Deliberately not a command. See the module note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankAssignment {
    /// Which node this is for.
    pub node: NodeId,
    /// Rank within the launch; 0 is the head and serves the API.
    pub rank: u16,
    /// How many nodes take part.
    pub world_size: u16,
    /// Address every rank rendezvouses on — the head's chosen address.
    pub master_addr: String,
    /// Port every rank rendezvouses on.
    pub master_port: u16,
    /// Which recipe to run, by id.
    pub recipe: String,
    /// Content hash of the recipe the head used, so a worker running a
    /// different revision refuses rather than silently launching something else.
    pub recipe_hash: String,
    /// Bounded overrides, still typed.
    ///
    /// Not flattened to strings: an integer setting would fail its own bound on
    /// the way back in, and the receiving rank re-validates against the same
    /// schema rather than trusting the head's word for it.
    pub settings: BTreeMap<String, atlasctl_protocol::settings::SettingValue>,
}

/// A whole multi-node launch, ready to be prepared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusterPlan {
    /// Identifies this attempt. A prepare from one epoch cannot be committed
    /// against another.
    pub epoch: String,
    /// The recipe being launched.
    pub recipe: String,
    /// Per-rank instructions, rank 0 first.
    pub ranks: Vec<RankAssignment>,
    /// The worst link class anywhere in the plan.
    pub link: LinkClass,
    /// Whether that link deserves a warning in the interface.
    pub link_warning: Option<String>,
}

/// Why a plan could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    /// The recipe needs a different number of nodes than were selected.
    #[error("{recipe} needs exactly {required} nodes, {selected} selected")]
    NodeCount {
        /// Recipe id.
        recipe: String,
        /// What the recipe requires.
        required: u32,
        /// What the user chose.
        selected: usize,
    },
    /// A selected node cannot run a model.
    #[error("{name} cannot run a model: {reason}")]
    NotLaunchable {
        /// Node display name.
        name: String,
        /// Why not.
        reason: String,
    },
    /// A selected node is not paired.
    #[error("{name} is not paired; pair it before launching on it")]
    NotPaired {
        /// Node display name.
        name: String,
    },
    /// The head has no address a collective could use.
    #[error("{name} has no usable network link for a cluster")]
    NoUsableLink {
        /// Node display name.
        name: String,
    },
    /// The head was not among the selected nodes.
    #[error("the head must be one of the selected nodes")]
    HeadNotSelected,
    /// The same node was selected twice.
    #[error("a node cannot take two ranks in one launch")]
    DuplicateNode,
}

/// Build a plan.
///
/// `epoch` is supplied by the caller rather than generated here so that
/// planning stays pure and therefore testable; the agent passes a random value.
///
/// # Errors
/// See [`PlanError`].
pub fn plan(
    recipe: &str,
    recipe_hash: &str,
    required_nodes: u32,
    selected: &[&NodeDescriptor],
    head: NodeId,
    settings: &BTreeMap<String, atlasctl_protocol::settings::SettingValue>,
    epoch: String,
) -> Result<ClusterPlan, PlanError> {
    if selected.len() as u32 != required_nodes {
        return Err(PlanError::NodeCount {
            recipe: recipe.to_owned(),
            required: required_nodes,
            selected: selected.len(),
        });
    }

    let mut seen = std::collections::BTreeSet::new();
    for n in selected {
        if !seen.insert(n.id) {
            return Err(PlanError::DuplicateNode);
        }
        if !matches!(n.pairing, atlasctl_protocol::fleet::PairingState::Paired) && !n.is_local {
            return Err(PlanError::NotPaired {
                name: n.name.to_string(),
            });
        }
        if !n.launchability.can_launch {
            return Err(PlanError::NotLaunchable {
                name: n.name.to_string(),
                reason: n.launchability.reason.clone(),
            });
        }
    }

    let head_node = selected
        .iter()
        .find(|n| n.id == head)
        .ok_or(PlanError::HeadNotSelected)?;
    // Not simply the head's best link. A DGX Spark carries several
    // point-to-point RoCE links, and `preferred_address` ranks by link *class*
    // — so with two RoCE ports it returns whichever sorts first, which is a
    // coin flip as to whether the workers are on it. Choosing wrong is not a
    // slow cluster but a hung one: the workers sit at the collective barrier
    // retrying a connection that will never complete.
    let master =
        rendezvous_address(head_node, selected).ok_or_else(|| PlanError::NoUsableLink {
            name: head_node.name.to_string(),
        })?;

    // The plan is only as good as its worst link: a cluster joined by one
    // ethernet hop runs at ethernet speed regardless of how fast the rest is.
    let worst = selected
        .iter()
        .filter_map(|n| n.preferred_address().map(|a| a.class))
        .min_by_key(|c| c.rank())
        .unwrap_or(LinkClass::Ethernet);

    let link_warning = worst.warns().then(|| {
        format!(
            "this cluster would run over {}, not RDMA. EP=2 decode is all-reduce bound, \
             so expect several times lower throughput than the published numbers.",
            worst.label()
        )
    });

    // Head first, then the rest in a stable order so two planners agree.
    let mut ordered: Vec<&NodeDescriptor> = Vec::with_capacity(selected.len());
    ordered.push(head_node);
    let mut others: Vec<&NodeDescriptor> =
        selected.iter().filter(|n| n.id != head).copied().collect();
    others.sort_by_key(|n| n.id);
    ordered.extend(others);

    let world_size = u16::try_from(ordered.len()).unwrap_or(u16::MAX);
    let ranks = ordered
        .iter()
        .enumerate()
        .map(|(i, n)| RankAssignment {
            node: n.id,
            rank: u16::try_from(i).unwrap_or(u16::MAX),
            world_size,
            master_addr: master.addr.clone(),
            master_port: DEFAULT_MASTER_PORT,
            recipe: recipe.to_owned(),
            recipe_hash: recipe_hash.to_owned(),
            settings: settings.clone(),
        })
        .collect();

    Ok(ClusterPlan {
        epoch,
        recipe: recipe.to_owned(),
        ranks,
        link: worst,
        link_warning,
    })
}

/// A fresh epoch, so a prepare from one attempt can never authorize a commit
/// from another.
///
/// Random rather than counted: a counter restarts at zero when the agent does,
/// and a replayed commit from before the restart would then match a new prepare.
#[must_use]
pub fn new_epoch() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS must supply entropy");
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Choose an address on the head that every worker shares a subnet with.
///
/// Preference order is the head's own — best link class first — so among
/// addresses the workers can all reach, the fastest still wins. Only when no
/// address is demonstrably shared does this fall back to the head's favourite,
/// because a worker whose subnets are unknown (paired but never dialled, or
/// running an older agent that does not report them) would otherwise make the
/// launch impossible rather than merely uncertain. The workers re-check
/// reachability at prepare, so a wrong guess is refused rather than hung.
#[must_use]
fn rendezvous_address<'a>(
    head: &'a NodeDescriptor,
    selected: &[&NodeDescriptor],
) -> Option<&'a atlasctl_protocol::fleet::NodeAddress> {
    let workers: Vec<&NodeDescriptor> = selected
        .iter()
        .copied()
        .filter(|n| n.id != head.id)
        .collect();

    // Same filter and ordering as `preferred_address`, because sharing a
    // subnet is a tiebreak between *usable* links, not a licence to promote an
    // unusable one. Without this a docker bridge wins: every machine has one on
    // the same private range, so it "shares a subnet" with everything, and the
    // collective would be pointed at a local bridge that reaches no other node.
    let shared = head
        .addresses
        .iter()
        .filter(|a| a.class.usable_for_cluster() && a.prefix_len > 0)
        .filter(|candidate| {
            workers.iter().all(|w| {
                w.addresses.iter().any(|a| {
                    a.class.usable_for_cluster()
                        && crate::rendezvous::shares_network(
                            &candidate.addr,
                            candidate.prefix_len,
                            &a.addr,
                        )
                })
            })
        })
        .max_by_key(|a| (a.class.rank(), a.speed_mbps.unwrap_or(0)));

    shared.or_else(|| head.preferred_address())
}

/// What a worker says when asked to prepare.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PrepareReply {
    /// Ready to commit.
    Prepared,
    /// Not ready, and will not become ready without intervention.
    Refused {
        /// Why, in words the user can act on.
        reason: String,
    },
}

/// Why a prepare was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefusalReason {
    /// The worker does not have this recipe.
    #[error("this node does not ship recipe {0}")]
    UnknownRecipe(String),
    /// The worker has the recipe, but a different revision of it.
    #[error(
        "recipe {recipe} differs between nodes: head has {head}, this node has {local}. \
         Update both to the same atlasctl before launching."
    )]
    RecipeMismatch {
        /// Recipe id.
        recipe: String,
        /// Hash the head sent.
        head: String,
        /// Hash this node computed.
        local: String,
    },
    /// Something is already running here.
    #[error("{0} is already running on this node")]
    AlreadyRunning(String),
    /// This node has already agreed to be a rank of a different launch.
    ///
    /// Distinct from `AlreadyRunning` because nothing has started: an operator
    /// told "already running" goes looking for a container that is not there.
    /// What they actually need to do is abandon the other launch.
    #[error(
        "this node is already reserved for a launch of {recipe}; \
         abort that launch, or wait for it to finish, before starting another"
    )]
    Reserved {
        /// Which recipe the outstanding reservation is for.
        recipe: String,
    },
    /// The rendezvous address is on a link this node is not attached to.
    ///
    /// Separate from every other refusal because it is the one an operator can
    /// act on directly, and because without it the failure is not a refusal at
    /// all -- the rank starts, waits at the collective barrier, and retries
    /// until somebody reads the logs.
    #[error("{0}")]
    RendezvousUnreachable(String),

    /// The container runtime is unavailable.
    #[error("the container runtime is not answering on this node")]
    DockerUnavailable,
    /// This node cannot launch at all.
    #[error("this node cannot run models: {0}")]
    NotLaunchable(String),
}

/// Tracks a prepare across every rank.
///
/// The commit gate lives here so there is exactly one place that decides
/// whether a cluster may start, rather than a condition repeated per call site.
#[derive(Debug, Clone)]
pub struct PrepareTracker {
    epoch: String,
    expected: Vec<NodeId>,
    replies: BTreeMap<NodeId, PrepareReply>,
}

impl PrepareTracker {
    /// Track the ranks in a plan.
    #[must_use]
    pub fn new(plan: &ClusterPlan) -> Self {
        Self {
            epoch: plan.epoch.clone(),
            expected: plan.ranks.iter().map(|r| r.node).collect(),
            replies: BTreeMap::new(),
        }
    }

    /// Record a reply. Replies for an unexpected node, or a stale epoch, are
    /// ignored rather than trusted.
    pub fn record(&mut self, epoch: &str, node: NodeId, reply: PrepareReply) -> bool {
        if epoch != self.epoch || !self.expected.contains(&node) {
            return false;
        }
        self.replies.insert(node, reply);
        true
    }

    /// Whether every rank has answered.
    #[must_use]
    pub fn complete(&self) -> bool {
        self.expected.iter().all(|n| self.replies.contains_key(n))
    }

    /// Nodes that refused, with their reasons.
    #[must_use]
    pub fn refusals(&self) -> Vec<(NodeId, String)> {
        self.replies
            .iter()
            .filter_map(|(n, r)| match r {
                PrepareReply::Refused { reason } => Some((*n, reason.clone())),
                PrepareReply::Prepared => None,
            })
            .collect()
    }

    /// Nodes that are ready and therefore hold a reservation to release if the
    /// launch is abandoned.
    #[must_use]
    pub fn prepared(&self) -> Vec<NodeId> {
        self.replies
            .iter()
            .filter(|(_, r)| matches!(r, PrepareReply::Prepared))
            .map(|(n, _)| *n)
            .collect()
    }

    /// Whether commit may proceed.
    ///
    /// Every rank must have answered, and every answer must be `Prepared`. A
    /// missing reply is not a yes.
    #[must_use]
    pub fn may_commit(&self) -> bool {
        self.complete() && self.refusals().is_empty()
    }
}
