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
use atlasctl_core::registry::RegistrySet;
use atlasctl_core::settings;
use atlasctl_protocol::PROTOCOL_VERSION;
use atlasctl_protocol::msg::{AgentError, ClientMsg, RecipeInfo, ServerMsg};

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
    /// Routes a control verb toward another machine.
    ///
    /// `None` means this agent cannot reach other machines: a verb aimed at
    /// one is answered with a typed `NotRoutable` rather than pretended.
    pub relay: Option<&'a dyn ControlRelay>,
}

/// A single client connection.
/// How long a completed exchange waits for a human decision.
///
/// Not a security bound — nothing is trusted while it waits — but an idle tab
/// should not be able to confirm words nobody is still looking at.
const PENDING_PAIRING_TTL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// An exchange that completed and is waiting to be accepted or refused.
struct PendingPairing {
    outcome: crate::fleet::PairOutcome,
    at: std::time::Instant,
}

pub struct Session<'a> {
    deps: SessionDeps<'a>,
    phase: Phase,
    /// The one exchange this session is holding, if any.
    ///
    /// Deliberately owned by the SESSION rather than by the fleet. A
    /// fleet-global map would let one browser confirm a ceremony another
    /// browser ran, and would need a race resolved between two tabs pairing the
    /// same node. Here the slot dies with the socket, so a browser that walks
    /// away mid-ceremony leaves nothing behind — and `Session::handle` is
    /// synchronous, so `PairPeer` and its decision are strictly ordered within
    /// a session and cannot race at all.
    ///
    /// One slot, replaced rather than accumulated: the UI shows one dialog, and
    /// replacing invalidates stale words for free.
    pending_pairing: Option<PendingPairing>,
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
                pending_pairing: None,
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
        // A verb aimed at another machine is routed here, before dispatch —
        // executing it on THIS machine instead would be the silent
        // misattribution the provenance fields exist to prevent. Only once
        // authenticated, so an unauthenticated socket still learns nothing.
        // Every arm below therefore handles only this machine's own verbs.
        if self.phase == Phase::Ready
            && let Some(replies) = self.route_remote(&msg)
        {
            return replies;
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
            (Phase::Ready, ClientMsg::ListRecipes { id, on: _ }) => {
                vec![ServerMsg::Recipes {
                    id,
                    recipes: self.inventory(),
                    // Answered locally over the browser socket: this
                    // machine's own answer, carried by no relay.
                    on: None,
                    via: None,
                }]
            }
            (
                Phase::Ready,
                ClientMsg::Preview {
                    id,
                    recipe,
                    settings,
                    on: _,
                },
            ) => self.preview(id, &recipe, &settings),
            (
                Phase::Ready,
                ClientMsg::Launch {
                    id,
                    recipe,
                    settings,
                    on: _,
                },
            ) => self.launch(id, &recipe, &settings),
            (Phase::Ready, ClientMsg::Stop { id, recipe, on: _ }) => self.stop(id, &recipe),
            (Phase::Ready, ClientMsg::Status { id, on: _ }) => self.status(id),

            (Phase::Ready, ClientMsg::ListNodes { id }) => self.nodes(id),
            // A watch is answered with the current fleet; the transport pushes
            // subsequent changes. Accepting the subscription here rather than in
            // the socket layer keeps the authorization decision in one place.
            (Phase::Ready, ClientMsg::WatchFleet { id, vitals: _ }) => self.nodes(id),
            (Phase::Ready, ClientMsg::PairPeer { id, node, code }) => self.pair(id, node, &code),
            (Phase::Ready, ClientMsg::PairPeerAt { id, target, code }) => {
                self.pair_at(id, &target, &code)
            }
            (
                Phase::Ready,
                ClientMsg::ConfirmPairing {
                    id,
                    node,
                    allow_control,
                },
            ) => self.confirm_pairing(id, node, allow_control),
            (Phase::Ready, ClientMsg::RejectPairing { id, node }) => self.reject_pairing(id, node),
            (Phase::Ready, ClientMsg::UnpairPeer { id, node }) => self.unpair(id, node),
            (Phase::Ready, ClientMsg::MintJoinCode { id, allow_control }) => {
                self.mint_join(id, allow_control)
            }
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

            (Phase::Ready, ClientMsg::LaunchStats { id, recipe, on: _ }) => {
                self.launch_stats(id, &recipe)
            }

            (
                Phase::Ready,
                ClientMsg::LaunchLogs {
                    id,
                    recipe,
                    lines,
                    on: _,
                },
            ) => self.launch_logs(id, &recipe, lines),

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

    /// The shared control core over this session's dependencies.
    ///
    /// Built per call, from borrows: the same [`crate::control::LocalControl`]
    /// the peer channel's terminal `Control` handler executes through, so a
    /// relayed verb cannot reach a check the local one skips or vice versa.
    fn control(&self) -> crate::control::LocalControl<'_> {
        crate::control::LocalControl {
            registry: self.deps.registry,
            launcher: self.deps.launcher,
            telemetry: self.deps.telemetry,
            can_launch: &self.deps.can_launch,
        }
    }

    /// Record denied-key attempts carried inside a settings refusal.
    ///
    /// The control core returns them typed in `BadSettings` rather than
    /// logging them itself, because only this session knows they came from
    /// the local browser — the caller the log exists to say something about.
    fn note_denied(&mut self, error: &AgentError) {
        if let AgentError::BadSettings { errors } = error {
            for e in errors {
                if let atlasctl_protocol::settings::SettingError::Denied { key, .. } = e {
                    self.denied_attempts.push(key.clone());
                }
            }
        }
    }

    fn inventory(&self) -> Vec<RecipeInfo> {
        self.control().recipes()
    }
}

fn err(id: Option<u32>, error: AgentError) -> ServerMsg {
    ServerMsg::Error { id, error }
}

mod fleet;

// The cluster-control trait the session drives. Split from this file on the
// 500-line cap, along the trait boundary: the seam is "what the session asks
// of a cluster", and nothing else moved.
#[path = "session/cluster_control.rs"]
mod cluster_control;
pub use cluster_control::ClusterControl;

#[path = "session/launch.rs"]
mod launch;

// The remote router and its relay trait. Split on the 500-line cap along the
// trust seam: where a verb GOES, versus what this machine does.
#[path = "session/remote.rs"]
mod remote;
pub use remote::ControlRelay;
pub mod telemetry;
pub use telemetry::LaunchTelemetry;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "session/fleet_fake.rs"]
mod fleet_fake;
#[cfg(test)]
#[path = "session/forward_tests.rs"]
mod forward_tests;
#[cfg(test)]
#[path = "session/join_tests.rs"]
mod join_tests;
#[cfg(test)]
#[path = "session/pair_at_tests.rs"]
mod pair_at_tests;
#[cfg(test)]
#[path = "session/pairing_tests.rs"]
mod pairing_tests;
