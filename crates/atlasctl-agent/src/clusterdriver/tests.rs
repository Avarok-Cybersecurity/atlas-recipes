// SPDX-License-Identifier: AGPL-3.0-only

//! Driving the two-phase launch through the orderings that produce a partial
//! cluster — the failure this design exists to prevent.
//!
//! Every test here is about what happens *after* something goes wrong, because
//! that is the part a real fleet exercises constantly and a happy-path test
//! never reaches.

use super::*;
use crate::cluster::RankAssignment;
use crate::fleet::PairOutcome;
use atlasctl_protocol::fleet::{
    DisplayName, Launchability, LinkClass, NodeAddress, NodeDescriptor, PairingState,
};
use std::sync::Mutex as StdMutex;

/// Every call a rank was asked to make, in order. Order is the point: a
/// rollback that releases the wrong rank, or releases before it has finished
/// asking, shows up here and nowhere else.
type Log = Arc<StdMutex<Vec<String>>>;

pub(super) fn node_id(seed: u8) -> NodeId {
    NodeId::parse(&format!("{:02x}", seed).repeat(32)).expect("64 hex chars")
}

fn descriptor(seed: u8, local: bool) -> NodeDescriptor {
    NodeDescriptor {
        id: node_id(seed),
        name: DisplayName::new(&format!("spark-{seed}")),
        is_local: local,
        pairing: PairingState::Paired,
        addresses: vec![NodeAddress {
            iface: "eth0".to_owned(),
            addr: format!("10.0.0.{seed}"),
            class: LinkClass::Roce,
            speed_mbps: Some(200_000),
            prefix_len: 30,
            rdma: true,
        }],
        launchability: Launchability::yes(),
        agent_version: "test".to_owned(),
        accelerator: "GB10".to_owned(),
        os: "Linux".to_owned(),
        vitals: None,
        alerts: Vec::new(),
        running: None,
        vouched_by: None,
        reached_via: None,
    }
}

pub(super) struct FixtureFleet(Vec<NodeDescriptor>);

impl FleetView for FixtureFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        self.0.clone()
    }
    fn pair<'a>(&'a self, _: NodeId, _: &'a str) -> crate::BoxFut<'a, anyhow::Result<PairOutcome>> {
        Box::pin(async move { unreachable!("the driver never pairs") })
    }
    fn pair_at<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
    ) -> crate::BoxFut<'a, anyhow::Result<PairOutcome>> {
        Box::pin(async move { unreachable!("the driver never pairs") })
    }
    fn trust(&self, _: &PairOutcome, _: bool) -> anyhow::Result<()> {
        unreachable!("the driver never pairs")
    }
    fn unpair(&self, _: NodeId) -> anyhow::Result<bool> {
        unreachable!("the driver never unpairs")
    }
}

/// This machine as a rank, recording what it was asked.
pub(super) struct FixtureRank {
    log: Log,
    prepare: PrepareReply,
    commit: Result<String, String>,
    stop: Result<(), String>,
    alive: bool,
    /// What this machine's copy of the recipe pins, as `recipe_port` answers.
    recipe_port: Option<u16>,
}

impl FixtureRank {
    fn ready(log: &Log) -> Self {
        Self {
            log: Arc::clone(log),
            prepare: PrepareReply::Prepared,
            commit: Ok("head-container".to_owned()),
            stop: Ok(()),
            alive: true,
            // The flagship recipes pin 8888, which is the case that made the
            // endpoint wrong: an operator who overrides nothing.
            recipe_port: Some(8888),
        }
    }
    fn note(&self, what: String) {
        self.log.lock().expect("log lock").push(what);
    }
}

impl RankService for FixtureRank {
    fn render(&self, a: &RankAssignment) -> anyhow::Result<(String, Vec<String>)> {
        self.note(format!("local.render(rank={})", a.rank));
        Ok((format!("docker run rank{}", a.rank), Vec::new()))
    }
    fn content_hash(&self, _: &str) -> anyhow::Result<String> {
        Ok("hash".to_owned())
    }
    fn recipe_port(&self, _: &str) -> anyhow::Result<Option<u16>> {
        Ok(self.recipe_port)
    }
    fn prepare(&self, epoch: &str, a: &RankAssignment) -> PrepareReply {
        self.note(format!("local.prepare(rank={})", a.rank));
        assert!(!epoch.is_empty(), "a prepare must carry an epoch");
        self.prepare.clone()
    }
    fn commit(&self, _: &str) -> anyhow::Result<String> {
        self.note("local.commit".to_owned());
        self.commit.clone().map_err(|e| anyhow::anyhow!(e))
    }
    fn alive(&self, container: &str) -> anyhow::Result<bool> {
        self.note(format!("local.alive({container})"));
        Ok(self.alive)
    }
    fn stop(&self, container: &str) -> anyhow::Result<()> {
        self.note(format!("local.stop({container})"));
        self.stop.clone().map_err(|e| anyhow::anyhow!(e))
    }
    fn abort(&self, _: &str) {
        self.note("local.abort".to_owned());
    }
}

/// Remote ranks, answering from a per-node script.
pub(super) struct FixtureTransport {
    log: Log,
    prepare: BTreeMap<NodeId, PrepareReply>,
    commit: BTreeMap<NodeId, Result<String, String>>,
    dead: BTreeMap<NodeId, bool>,
    /// Ranks killed mid-run by a test, after commit succeeded.
    killed: StdMutex<std::collections::BTreeSet<NodeId>>,
}

impl FixtureTransport {
    fn new(log: &Log) -> Self {
        Self {
            log: Arc::clone(log),
            prepare: BTreeMap::new(),
            commit: BTreeMap::new(),
            dead: BTreeMap::new(),
            killed: StdMutex::new(std::collections::BTreeSet::new()),
        }
    }
    /// A peer whose container does not survive its own start.
    pub(super) fn dying(mut self, node: NodeId) -> Self {
        self.dead.insert(node, true);
        self
    }
    pub(super) fn refusing(mut self, node: NodeId, why: &str) -> Self {
        self.prepare.insert(
            node,
            PrepareReply::Refused {
                reason: why.to_owned(),
            },
        );
        self
    }
    pub(super) fn failing_commit(mut self, node: NodeId, why: &str) -> Self {
        self.commit.insert(node, Err(why.to_owned()));
        self
    }
    fn note(&self, what: String) {
        self.log.lock().expect("log lock").push(what);
    }
}

impl RankTransport for FixtureTransport {
    fn preview<'a>(
        &'a self,
        node: NodeId,
        _: SocketAddr,
        a: &'a RankAssignment,
    ) -> crate::BoxFut<'a, anyhow::Result<(String, Vec<String>)>> {
        Box::pin(async move {
            self.note(format!("{}.preview(rank={})", node.short(), a.rank));
            Ok((format!("docker run rank{}", a.rank), Vec::new()))
        })
    }

    fn prepare<'a>(
        &'a self,
        node: NodeId,
        _: SocketAddr,
        _: &'a str,
        a: &'a RankAssignment,
    ) -> crate::BoxFut<'a, PrepareReply> {
        Box::pin(async move {
            self.note(format!("{}.prepare(rank={})", node.short(), a.rank));
            self.prepare
                .get(&node)
                .cloned()
                .unwrap_or(PrepareReply::Prepared)
        })
    }

    fn commit<'a>(
        &'a self,
        node: NodeId,
        _: SocketAddr,
        _: &'a str,
    ) -> crate::BoxFut<'a, anyhow::Result<String>> {
        Box::pin(async move {
            self.note(format!("{}.commit", node.short()));
            self.commit
                .get(&node)
                .cloned()
                .unwrap_or_else(|| Ok(format!("{}-container", node.short())))
                .map_err(|e| anyhow::anyhow!(e))
        })
    }

    fn abort<'a>(&'a self, node: NodeId, _: SocketAddr, _: &'a str) -> crate::BoxFut<'a, ()> {
        Box::pin(async move {
            self.note(format!("{}.abort", node.short()));
        })
    }

    fn alive<'a>(
        &'a self,
        node: NodeId,
        _: SocketAddr,
        container: &'a str,
    ) -> crate::BoxFut<'a, anyhow::Result<bool>> {
        Box::pin(async move {
            self.note(format!("{}.alive({container})", node.short()));
            if self.killed.lock().expect("killed lock").contains(&node) {
                return Ok(false);
            }
            Ok(!self.dead.get(&node).copied().unwrap_or(false))
        })
    }

    fn stop<'a>(
        &'a self,
        node: NodeId,
        _: SocketAddr,
        container: &'a str,
    ) -> crate::BoxFut<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.note(format!("{}.stop({container})", node.short()));
            // A killed rank is one this transport cannot reach, which is exactly
            // the case that used to be reported as a successful stop.
            if self.killed.lock().expect("killed lock").contains(&node) {
                anyhow::bail!("{} is unreachable", node.short());
            }
            Ok(())
        })
    }

    fn kill_for_test(&self, node: NodeId) {
        self.killed.lock().expect("killed lock").insert(node);
    }
}

/// A fresh call log.
pub(super) fn new_log() -> Log {
    Arc::new(StdMutex::new(Vec::new()))
}

/// This machine, ready to be a rank.
pub(super) fn ready_rank(log: &Log) -> FixtureRank {
    FixtureRank::ready(log)
}

/// This machine, refusing to be a rank.
pub(super) fn refusing_rank(log: &Log, why: &str) -> FixtureRank {
    // Built by difference from the ready fixture, so a new field on the
    // service does not have to be restated in four places to compile.
    FixtureRank {
        prepare: PrepareReply::Refused {
            reason: why.to_owned(),
        },
        commit: Err("was never prepared".to_owned()),
        ..FixtureRank::ready(log)
    }
}

/// This machine, unable to stop what it started.
pub(super) fn refusing_stop_rank(log: &Log) -> FixtureRank {
    FixtureRank {
        stop: Err("the container runtime is not answering".to_owned()),
        ..FixtureRank::ready(log)
    }
}

/// This machine, whose container dies moments after it starts.
pub(super) fn dying_rank(log: &Log) -> FixtureRank {
    FixtureRank {
        alive: false,
        ..FixtureRank::ready(log)
    }
}

/// Peers that accept unless told otherwise.
pub(super) fn transport(log: &Log) -> FixtureTransport {
    FixtureTransport::new(log)
}

/// A three-node fleet: this machine plus two peers.
pub(super) fn driver(rank: FixtureRank, transport: FixtureTransport) -> (ClusterDriver, Log) {
    let log = Arc::clone(&rank.log);
    let fleet = FixtureFleet(vec![
        descriptor(1, true),
        descriptor(2, false),
        descriptor(3, false),
    ]);
    (
        ClusterDriver::new(Arc::new(fleet), Arc::new(rank), Arc::new(transport), 34334)
            // The fixtures answer liveness from a table rather than from a real
            // container, so there is nothing for a settling window to observe.
            .with_settle(std::time::Duration::ZERO),
        log,
    )
}

pub(super) fn recipe() -> RecipeId {
    RecipeId::parse("some-recipe").expect("a valid id")
}

pub(super) fn all_three() -> Vec<NodeId> {
    vec![node_id(1), node_id(2), node_id(3)]
}

pub(super) fn calls(log: &Log) -> Vec<String> {
    log.lock().expect("log lock").clone()
}

/// The fallback endpoint port must be the engine's own default, not a number
/// that once looked like it.
///
/// It was 8000 for as long as this constant existed, and the engine has never
/// defaulted to 8000. Nothing caught it because the only thing asserting the
/// value was a doc comment claiming it was "the serving runtime's own default"
/// -- and a comment cannot fail. The symptom is the one `serve_port`'s doc
/// already describes for the layer above: an endpoint printed as `:8000` while
/// rank 0 serves on `:8888`.
///
/// Asserted against the vendored snapshot, which is reflected out of the
/// engine's clap, so the engine changing its default fails here rather than
/// quietly making this wrong again.
#[test]
fn the_fallback_port_is_the_engines_own_default() {
    use super::plan::DEFAULT_SERVE_PORT;
    const SNAPSHOT: &str = include_str!("../../../../vendor/serve-options.v1.json");

    // Deliberately a string scan rather than a JSON parse: this file has no
    // serde dependency, and the shape being matched is the one the generator
    // emits verbatim.
    let port_entry = SNAPSHOT
        .split('{')
        .find(|chunk| chunk.contains("\"key\": \"port\""))
        .expect("the snapshot has a `port` flag");
    let default = port_entry
        .split("\"default\":")
        .nth(1)
        .and_then(|rest| rest.split('"').nth(1))
        .expect("the `port` flag declares a default");

    assert_eq!(
        default,
        DEFAULT_SERVE_PORT.to_string(),
        "DEFAULT_SERVE_PORT is {DEFAULT_SERVE_PORT}, but the engine defaults --port to {default}. \
         The endpoint this decorates would tell an operator the wrong port."
    );
}
