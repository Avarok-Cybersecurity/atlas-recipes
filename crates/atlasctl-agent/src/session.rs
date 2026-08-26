// SPDX-License-Identifier: AGPL-3.0-only

//! One client connection, as a state machine.
//!
//! Deliberately transport-free: it takes a decoded message and returns messages
//! to send. That keeps every security property — handshake ordering, token
//! checking, recipe validation, settings bounds — testable without opening a
//! socket, and it means the websocket layer has no decisions of its own to get
//! wrong.

use crate::launcher::Launcher;
use crate::token;
use atlasctl_core::registry::{RecipeRef, RegistrySet};
use atlasctl_core::settings;
use atlasctl_protocol::msg::{AgentError, ClientMsg, RecipeInfo, ServerMsg};
use atlasctl_protocol::settings::SettingValue;
use atlasctl_protocol::{PROTOCOL_VERSION, RecipeId};
use std::collections::BTreeMap;

/// Where a connection has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the client's hello.
    AwaitingHello,
    /// Authenticated; normal traffic allowed.
    Ready,
    /// Finished, for whatever reason.
    Closed,
}

/// Everything a session needs from the outside world.
pub struct SessionDeps<'a> {
    /// The recipe inventory.
    pub registry: &'a RegistrySet,
    /// How launches actually happen.
    pub launcher: &'a dyn Launcher,
    /// The expected pairing token.
    pub token: &'a str,
    /// Whether this machine can run a recipe at all.
    pub can_launch: Result<(), String>,
    /// What this agent knows about other machines.
    ///
    /// `None` is a single-node agent: the fleet verbs answer with this machine
    /// alone rather than erroring, because "no fleet" is a normal state and a
    /// page that got an error would show a fault where there is none.
    pub fleet: Option<&'a dyn crate::fleet::FleetView>,
    /// Renders a cluster preview by asking each rank in turn.
    ///
    /// `None` means cluster launches are unavailable on this agent, which is
    /// answered plainly rather than by pretending.
    pub cluster: Option<&'a dyn ClusterControl>,
    /// Sampling a running launch, when this agent can.
    pub telemetry: Option<&'a dyn LaunchTelemetry>,
    /// The window in which one new machine may join.
    ///
    /// `None` on an agent that cannot take members — there is nothing to open,
    /// and saying so is better than minting a code nothing will honour.
    pub joining: Option<&'a crate::joining::JoinWindow>,
}

/// Asks every rank of a planned cluster what it would run.
///
/// A trait so the session stays transport-free: the preview needs to reach
/// other machines, and a session that opened sockets itself could not be tested
/// without them.
pub trait ClusterControl: Send + Sync {
    /// Plan the launch and collect each rank's own rendering. Reserves nothing.
    ///
    /// # Errors
    /// If the plan is impossible, or a rank refuses or cannot be reached.
    fn preview(
        &self,
        recipe: &RecipeId,
        nodes: &[atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<
        (
            Vec<atlasctl_protocol::msg::fleet::RankPreview>,
            Option<String>,
        ),
        String,
    >;

    /// Ask every rank to validate and reserve. Nothing starts.
    ///
    /// Returns the epoch a later commit must quote, each rank's answer, and
    /// whether a commit may proceed. A rank refusing is a normal outcome
    /// reported in the answers, not an error.
    ///
    /// # Errors
    /// If the plan itself is impossible, which is before any rank was asked.
    fn prepare(
        &self,
        recipe: &RecipeId,
        nodes: &[atlasctl_protocol::fleet::NodeId],
        head: atlasctl_protocol::fleet::NodeId,
        settings: &BTreeMap<String, SettingValue>,
    ) -> Result<
        (
            String,
            Vec<atlasctl_protocol::msg::fleet::RankPrepare>,
            bool,
        ),
        String,
    >;

    /// Start what every rank prepared under this epoch.
    ///
    /// # Errors
    /// If no such prepare is outstanding, or a rank fails to start — in which
    /// case every rank that did start has already been stopped.
    fn commit(
        &self,
        epoch: &str,
    ) -> Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>;

    /// Abandon a prepare, releasing every reservation.
    fn abort(&self, epoch: &str);

    /// Stop every rank of the cluster this agent started.
    ///
    /// # Errors
    /// If no cluster is running, or a rank could not be stopped — and the
    /// error names which, because a rank left running holds a whole GPU.
    fn stop_cluster(&self) -> Result<Vec<atlasctl_protocol::msg::fleet::RankStarted>, String>;

    /// Check a running cluster is still whole, and tear it down if it is not.
    ///
    /// The settle gate at commit is a liveness check by construction: weights
    /// take minutes to load, so it cannot wait for readiness and only catches
    /// a rank that dies immediately. A rank that dies four minutes in — during
    /// model build, say — passed that gate, and the survivors then hold their
    /// GPUs indefinitely serving nothing, because a half cluster waits at a
    /// rendezvous that will never complete.
    ///
    /// Returns a description of what it tore down, or `None` when the cluster
    /// is whole or absent.
    fn supervise(&self) -> Option<String>;
}

/// A single client connection.
pub struct Session<'a> {
    deps: SessionDeps<'a>,
    phase: Phase,
    /// Denied-key attempts, which the caller logs. Nothing in a legitimate UI
    /// offers a denied key, so an attempt says something about the client.
    pub denied_attempts: Vec<String>,
}

impl<'a> Session<'a> {
    /// Open a session. The agent speaks first.
    pub fn new(deps: SessionDeps<'a>) -> (Self, ServerMsg) {
        let welcome = ServerMsg::Welcome {
            protocol_min: PROTOCOL_VERSION,
            protocol_max: PROTOCOL_VERSION,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
        };
        (
            Self {
                deps,
                phase: Phase::AwaitingHello,
                denied_attempts: Vec::new(),
            },
            welcome,
        )
    }

    /// Whether the session is finished.
    pub fn is_closed(&self) -> bool {
        self.phase == Phase::Closed
    }

    /// A frame that failed to deserialize.
    ///
    /// Reported rather than ignored, and it ends the session: a client sending
    /// something we cannot parse is not one we should keep guessing for.
    pub fn on_malformed(&mut self, detail: String) -> ServerMsg {
        self.phase = Phase::Closed;
        ServerMsg::Error {
            id: None,
            error: AgentError::InvalidMessage { detail },
        }
    }

    /// Whether the handshake has completed.
    ///
    /// The transport asks before forwarding a pushed frame: fleet events name
    /// machines on someone's network, and an unauthenticated socket must not
    /// receive them just because it stayed open.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.phase, Phase::Ready)
    }

    /// Handle one decoded message.
    pub fn handle(&mut self, msg: ClientMsg) -> Vec<ServerMsg> {
        if self.phase == Phase::Closed {
            return Vec::new();
        }
        match (&self.phase, msg) {
            (
                Phase::AwaitingHello,
                ClientMsg::Hello {
                    protocol_version,
                    token,
                },
            ) => self.hello(protocol_version, &token),
            // Nothing else is answered before the handshake, so an unauthorized
            // client cannot even enumerate what recipes exist.
            (Phase::AwaitingHello, _) => {
                self.phase = Phase::Closed;
                vec![ServerMsg::Error {
                    id: None,
                    error: AgentError::NotReady,
                }]
            }
            (Phase::Ready, ClientMsg::Hello { .. }) => {
                vec![err(
                    None,
                    AgentError::InvalidMessage {
                        detail: "already greeted".into(),
                    },
                )]
            }
            (Phase::Ready, ClientMsg::ListRecipes { id }) => {
                vec![ServerMsg::Recipes {
                    id,
                    recipes: self.inventory(),
                }]
            }
            (
                Phase::Ready,
                ClientMsg::Preview {
                    id,
                    recipe,
                    settings,
                },
            ) => self.preview(id, &recipe, &settings),
            (
                Phase::Ready,
                ClientMsg::Launch {
                    id,
                    recipe,
                    settings,
                },
            ) => self.launch(id, &recipe, &settings),
            (Phase::Ready, ClientMsg::Stop { id, recipe }) => self.stop(id, &recipe),
            (Phase::Ready, ClientMsg::Status { id }) => self.status(id),

            (Phase::Ready, ClientMsg::ListNodes { id }) => self.nodes(id),
            // A watch is answered with the current fleet; the transport pushes
            // subsequent changes. Accepting the subscription here rather than in
            // the socket layer keeps the authorization decision in one place.
            (Phase::Ready, ClientMsg::WatchFleet { id, vitals: _ }) => self.nodes(id),
            (Phase::Ready, ClientMsg::PairPeer { id, node, code }) => self.pair(id, node, &code),
            (Phase::Ready, ClientMsg::UnpairPeer { id, node }) => self.unpair(id, node),
            (Phase::Ready, ClientMsg::MintJoinCode { id }) => self.mint_join(id),
            (Phase::Ready, ClientMsg::RevokeJoinCode { id }) => self.revoke_join(id),

            // A preview is rendered by each rank in turn, on the machine that
            // would run it — never invented here. The head does not know what
            // recipe revision or hardware the other machine has.
            (
                Phase::Ready,
                ClientMsg::PreviewCluster {
                    id,
                    recipe,
                    nodes,
                    head,
                    settings,
                },
            ) => self.preview_cluster(id, &recipe, &nodes, head, &settings),

            // Two phases, because a single-phase launch cannot fail cleanly:
            // the third machine's refusal would leave two containers waiting
            // forever on a rendezvous that will never complete.
            (
                Phase::Ready,
                ClientMsg::PrepareCluster {
                    id,
                    recipe,
                    nodes,
                    head,
                    settings,
                },
            ) => self.prepare_cluster(id, &recipe, &nodes, head, &settings),

            (Phase::Ready, ClientMsg::CommitCluster { id, epoch }) => {
                self.commit_cluster(id, &epoch)
            }

            (Phase::Ready, ClientMsg::AbortCluster { id, epoch }) => self.abort_cluster(id, &epoch),

            (Phase::Ready, ClientMsg::StopCluster { id }) => self.stop_cluster(id),

            (Phase::Ready, ClientMsg::LaunchStats { id, recipe }) => self.launch_stats(id, &recipe),

            (Phase::Ready, ClientMsg::LaunchLogs { id, recipe, lines }) => {
                self.launch_logs(id, &recipe, lines)
            }

            (Phase::Closed, _) => Vec::new(),
        }
    }

    fn hello(&mut self, version: u32, presented: &str) -> Vec<ServerMsg> {
        if version != PROTOCOL_VERSION {
            self.phase = Phase::Closed;
            return vec![err(
                None,
                AgentError::UnsupportedProtocol {
                    min: PROTOCOL_VERSION,
                    max: PROTOCOL_VERSION,
                    requested: version,
                },
            )];
        }
        if !token::matches(self.deps.token, presented) {
            self.phase = Phase::Closed;
            return vec![err(None, AgentError::NotPaired)];
        }
        self.phase = Phase::Ready;
        let (can_launch, reason) = match &self.deps.can_launch {
            Ok(()) => (true, None),
            Err(why) => (false, Some(why.clone())),
        };
        vec![ServerMsg::Ready {
            protocol_version: PROTOCOL_VERSION,
            schema: settings::schema(),
            recipes: self.inventory(),
            can_launch,
            can_launch_reason: reason,
        }]
    }

    /// Resolve a recipe id against the compiled-in set.
    ///
    /// The id is already syntactically valid — it could not have been
    /// deserialized otherwise — so this only answers "does it exist here".
    fn resolve(&self, id: &RecipeId) -> Result<atlasctl_core::Recipe, AgentError> {
        self.deps
            .registry
            .resolve(&RecipeRef::Bare(id.as_str().to_string()))
            .map_err(|_| AgentError::UnknownRecipe {
                recipe: id.to_string(),
            })
    }

    /// Check requested settings, recording any denied-key attempts.
    fn check_settings(
        &mut self,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Result<BTreeMap<String, atlasctl_core::ScalarValue>, AgentError> {
        settings::validate(requested).map_err(|errors| {
            for e in &errors {
                if let atlasctl_protocol::settings::SettingError::Denied { key, .. } = e {
                    self.denied_attempts.push(key.clone());
                }
            }
            AgentError::BadSettings { errors }
        })
    }

    fn preview(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        let recipe = match self.resolve(recipe_id) {
            Ok(r) => r,
            Err(e) => return vec![err(Some(id), e)],
        };
        let overrides = match self.check_settings(requested) {
            Ok(o) => o,
            Err(e) => return vec![err(Some(id), e)],
        };
        match self.deps.launcher.preview(&recipe, &overrides) {
            Ok(p) => vec![ServerMsg::Preview {
                id,
                command: p.command,
                unapplied: p.unapplied,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    fn launch(
        &mut self,
        id: u32,
        recipe_id: &RecipeId,
        requested: &BTreeMap<String, SettingValue>,
    ) -> Vec<ServerMsg> {
        if let Err(why) = &self.deps.can_launch {
            return vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe_id.clone(),
                    reason: why.clone(),
                },
            )];
        }
        let recipe = match self.resolve(recipe_id) {
            Ok(r) => r,
            Err(e) => return vec![err(Some(id), e)],
        };
        if let Err(why) = recipe.launchable() {
            return vec![err(
                Some(id),
                AgentError::NotLaunchable {
                    recipe: recipe_id.clone(),
                    reason: why.to_string(),
                },
            )];
        }
        let overrides = match self.check_settings(requested) {
            Ok(o) => o,
            Err(e) => return vec![err(Some(id), e)],
        };
        match self.deps.launcher.launch(&recipe, &overrides) {
            Ok(started) => vec![ServerMsg::Started {
                id,
                recipe: recipe_id.clone(),
                container: started.container,
                endpoint: started.endpoint,
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    fn stop(&mut self, id: u32, recipe_id: &RecipeId) -> Vec<ServerMsg> {
        match self.deps.launcher.stop(recipe_id.as_str()) {
            Ok(()) => vec![ServerMsg::Stopped {
                id,
                recipe: recipe_id.clone(),
            }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    fn status(&mut self, id: u32) -> Vec<ServerMsg> {
        match self.deps.launcher.running() {
            Ok(running) => vec![ServerMsg::Status { id, running }],
            Err(e) => vec![err(Some(id), e)],
        }
    }

    fn inventory(&self) -> Vec<RecipeInfo> {
        self.deps
            .registry
            .list()
            .into_iter()
            .filter_map(|entry| {
                let id = RecipeId::parse(&entry.name).ok()?;
                let r = self
                    .deps
                    .registry
                    .resolve(&RecipeRef::Bare(entry.name))
                    .ok()?;
                let (runnable, reason) = match r.launchable() {
                    Ok(()) => (true, None),
                    Err(why) => (false, Some(why.to_string())),
                };
                Some(RecipeInfo {
                    id,
                    model: r.model.clone(),
                    nodes: r.topology.min_nodes,
                    runnable,
                    reason,
                    defaults: r
                        .defaults
                        .iter()
                        .map(|(k, v)| (k.clone(), to_wire(v)))
                        .collect(),
                })
            })
            .collect()
    }
}

fn to_wire(v: &atlasctl_core::ScalarValue) -> SettingValue {
    use atlasctl_core::ScalarValue as S;
    match v {
        S::Bool(b) => SettingValue::Bool(*b),
        S::Int(i) => SettingValue::Int(*i),
        S::Float(f) => SettingValue::Float(*f),
        S::Str(s) => SettingValue::Str(s.clone()),
    }
}

fn err(id: Option<u32>, error: AgentError) -> ServerMsg {
    ServerMsg::Error { id, error }
}

mod fleet;
pub mod telemetry;
pub use telemetry::LaunchTelemetry;

#[cfg(test)]
mod tests;
