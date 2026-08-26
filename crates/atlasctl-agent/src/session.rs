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

            // Cluster verbs need the peer channel, which is not connected yet.
            // Refusing plainly is the honest answer: a preview rendered without
            // asking the other ranks would be a guess presented as a fact, and a
            // prepare that pretended to reserve would be worse.
            (
                Phase::Ready,
                ClientMsg::PreviewCluster { id, .. }
                | ClientMsg::PrepareCluster { id, .. }
                | ClientMsg::CommitCluster { id, .. }
                | ClientMsg::AbortCluster { id, .. },
            ) => vec![err(Some(id), AgentError::NotReady)],

            (Phase::Closed, _) => Vec::new(),
        }
    }

    /// The fleet, local node first.
    fn nodes(&self, id: u32) -> Vec<ServerMsg> {
        let nodes = self
            .deps
            .fleet
            .map(crate::fleet::FleetView::nodes)
            .unwrap_or_default();
        vec![ServerMsg::Nodes { id, nodes }]
    }

    fn pair(
        &mut self,
        id: u32,
        node: atlasctl_protocol::fleet::NodeId,
        code: &str,
    ) -> Vec<ServerMsg> {
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

    fn unpair(&mut self, id: u32, node: atlasctl_protocol::fleet::NodeId) -> Vec<ServerMsg> {
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

#[cfg(test)]
mod tests;
