// SPDX-License-Identifier: AGPL-3.0-only

//! The loopback websocket listener.
//!
//! This layer makes no decisions. It binds loopback, applies the guard before
//! the upgrade completes, decodes frames, and hands them to a [`Session`].
//! Everything that could be got wrong lives in modules that are testable
//! without a socket; keeping the transport thin is what makes that true.

use crate::guard;
use crate::launcher::Launcher;
use crate::session::{Session, SessionDeps};
use anyhow::{Context, Result};
use atlasctl_core::registry::RegistrySet;
use atlasctl_protocol::msg::{ClientMsg, ServerMsg};
use axum::Router;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade, ws};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

/// Largest frame we will decode.
///
/// Every legitimate message is small. A cap means a client cannot make the
/// agent allocate on its say-so.
const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Shared state for the listener.
pub struct AgentState {
    /// The recipe inventory.
    pub registry: RegistrySet,
    /// How launches happen.
    pub launcher: Box<dyn Launcher>,
    /// The expected pairing token.
    pub token: String,
    /// Whether this machine can run a recipe, and why not if it cannot.
    pub can_launch: Result<(), String>,
    /// What this machine's accelerator reports itself to be, for the launch
    /// path's hardware-aware refusal. Empty when the probe found nothing.
    pub accelerator: String,
    /// The window in which one new machine may join, when this agent has one.
    pub joining: Option<Arc<crate::joining::JoinWindow>>,
    /// Port we are listening on, for the Host check.
    pub port: u16,
    /// Whether development origins are accepted.
    pub allow_dev_origins: bool,
    /// What this agent knows about other machines.
    ///
    /// `None` for a single-node agent, which is a normal configuration rather
    /// than a degraded one.
    pub fleet: Option<Box<dyn crate::fleet::FleetView>>,
    /// Renders a cluster preview by asking each rank.
    ///
    /// `None` on an agent that cannot reach peers, which is answered plainly
    /// rather than by inventing a preview.
    pub cluster: Option<std::sync::Arc<dyn crate::session::ClusterControl>>,
    /// Sampling a running launch, when this agent can.
    pub telemetry: Option<Box<dyn crate::session::LaunchTelemetry>>,
    /// Routes a control verb toward another machine.
    ///
    /// `None` on an agent with no peer transport, which answers such verbs
    /// with a typed refusal rather than pretending.
    pub relay: Option<std::sync::Arc<dyn crate::session::ControlRelay>>,
    /// Fleet changes pushed to every authenticated session.
    ///
    /// A broadcast channel rather than a per-session queue: a slow tab must not
    /// be able to stall the sampler for the others. Lagging receivers lose the
    /// oldest frames, which for coalesced vitals costs nothing.
    pub events: tokio::sync::broadcast::Sender<ServerMsg>,
}

/// Build the router.
pub fn router(state: Arc<AgentState>) -> Router {
    Router::new().route("/ws", get(upgrade)).with_state(state)
}

/// Bind loopback and serve until the process is asked to stop.
///
/// The bind address is a literal, never a hostname: resolving a name could
/// yield something that is not loopback, and this listener must never be
/// reachable from the network.
pub async fn serve(state: Arc<AgentState>, port: u16) -> Result<()> {
    serve_on(state, bind(port).await?).await
}

/// Claim the browser port, or say why not.
///
/// Separate from [`serve_on`] so the caller can bind *before* it prints
/// anything. `agent run` used to announce "listening on 127.0.0.1:PORT", the
/// docker status and the full pairing token, and only then try to bind — so a
/// port conflict produced a complete, confident success banner followed by
/// `could not bind`. The operator had already been handed a token for an agent
/// that does not exist.
///
/// # Errors
/// If the port is taken.
pub async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(addr).await.with_context(|| {
        // Naming the remedy, not just the symptom. This is what an
        // operator sees after `curl … | sh` on a machine that already had
        // an agent: the installer could not bootstrap the service (the
        // label was loaded), suggested `agent install` to find out why,
        // and that reproduced the same line — so `agent run` by hand was
        // the third dead end in a row.
        format!(
            "could not bind {addr} — an agent is already listening there.\n\
                 \x20 If you meant to upgrade it, `atlasctl agent install` now replaces\n\
                 \x20 a running service in place. If you started one by hand, stop that\n\
                 \x20 process first, or pass `--port` to run a second one alongside it."
        )
    })?;

    let bound = listener.local_addr()?;
    debug_assert!(
        bound.ip().is_loopback(),
        "the browser listener must be loopback-only"
    );
    Ok(listener)
}

/// Serve on a listener the caller already holds.
///
/// # Errors
/// If the listener stops unexpectedly.
pub async fn serve_on(state: Arc<AgentState>, listener: tokio::net::TcpListener) -> Result<()> {
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    .context("the agent listener stopped unexpectedly")
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Decide whether a connection may upgrade, then run a session on it.
async fn upgrade(
    State(state): State<Arc<AgentState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // Belt and braces: the socket is bound to loopback, so a non-loopback peer
    // should be impossible. If one appears, something is wrong enough to refuse.
    if !peer.ip().is_loopback() {
        return (StatusCode::FORBIDDEN, "not a loopback connection").into_response();
    }

    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Err(refusal) = guard::check(
        header("origin"),
        header("host"),
        state.port,
        state.allow_dev_origins,
    ) {
        // Refused before the upgrade completes, so a rejected page never gets a
        // websocket at all.
        return (StatusCode::FORBIDDEN, refusal.to_string()).into_response();
    }

    upgrade
        .max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| run_session(socket, state))
}

async fn run_session(mut socket: ws::WebSocket, state: Arc<AgentState>) {
    let mut events = state.events.subscribe();
    let (mut session, welcome) = Session::new(SessionDeps {
        registry: &state.registry,
        launcher: state.launcher.as_ref(),
        token: &state.token,
        can_launch: state.can_launch.clone(),
        accelerator: &state.accelerator,
        fleet: state.fleet.as_deref(),
        cluster: state.cluster.as_deref(),
        telemetry: state.telemetry.as_deref(),
        joining: state.joining.as_deref(),
        relay: state.relay.as_deref(),
    });

    if send(&mut socket, &welcome).await.is_err() {
        return;
    }

    loop {
        let frame = tokio::select! {
            // Pushed fleet changes. Only forwarded once the handshake has
            // completed — an unauthenticated socket learns nothing about the
            // machines on this network.
            pushed = events.recv() => {
                match pushed {
                    Ok(msg) => {
                        if session.is_ready() && send(&mut socket, &msg).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    // Lagged: this tab fell behind and lost some samples. The
                    // next one supersedes them, so carry on rather than
                    // dropping the connection.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(f)) => f,
                    _ => break,
                }
            }
        };

        let text = match frame {
            ws::Message::Text(t) => t,
            // Binary frames are not part of this protocol. Accepting them would
            // widen the surface for nothing.
            ws::Message::Binary(_) => {
                let _ = send(
                    &mut socket,
                    &session.on_malformed("binary frames are not accepted".into()),
                )
                .await;
                break;
            }
            ws::Message::Close(_) => break,
            _ => continue,
        };

        let replies = match serde_json::from_str::<ClientMsg>(&text) {
            Ok(msg) => session.handle(msg),
            Err(e) => vec![session.on_malformed(e.to_string())],
        };

        for reply in replies {
            if send(&mut socket, &reply).await.is_err() {
                return;
            }
        }

        // Denied keys are logged rather than merely refused: nothing in a real
        // client offers one, so an attempt says something about the caller.
        for key in session.denied_attempts.drain(..) {
            eprintln!("atlasctl agent: client attempted to set the denied key `{key}`");
        }

        if session.is_closed() {
            break;
        }
    }

    // Dropping the socket closes it; axum's WebSocket has no explicit close.
    let _ = socket.send(ws::Message::Close(None)).await;
}

async fn send(socket: &mut ws::WebSocket, msg: &ServerMsg) -> Result<()> {
    let text = serde_json::to_string(msg).context("encoding a reply")?;
    socket
        .send(ws::Message::Text(text.into()))
        .await
        .context("sending a reply")
}

#[cfg(test)]
mod tests;
