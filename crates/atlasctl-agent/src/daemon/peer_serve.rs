// SPDX-License-Identifier: AGPL-3.0-only

//! Serving a pinned peer's frames, split from `daemon.rs` for size.
//!
//! The seam is the dispatch boundary: `daemon.rs` owns listeners, spawn
//! loops and lifecycle, and this file owns what a frame from an
//! already-authenticated pinned peer is answered with — the rank verbs, the
//! terminal `Control` executor (rules T1–T4) and the `ControlTo` relay
//! (rules R1–R6).
//!
//! The no-nesting rule is structural here, not checked: [`terminal_control`]
//! has no dialer, no fleet and no address book in scope, so code that
//! forwards from inside it cannot be written without changing its signature
//! in review; and [`relay_control`] is the only place that dials, and the
//! only frame it ever writes onward is the terminal `Control` — which itself
//! cannot name anywhere further to go.

use crate::control::{ControlHost, LocalControl};
use crate::identity::{Identity, PinStore};
use crate::peer::control::send_control;
use crate::peer::wire::{PeerFrame, read_frame, write_frame};
use crate::rank::RankService;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::{AgentError, ControlRep, ControlReq};
use std::sync::Arc;
use std::time::Duration;

/// Everything the pinned-peer serving path needs, bundled so a listener
/// cannot be wired with half of it (PCND — every field is required).
pub(crate) struct PeerServe {
    /// This agent's own keypair; also names WHO refused in a refusal —
    /// "dgx1 could not reach dgx3" and "dgx3 said no" send the operator to
    /// different machines.
    pub identity: Arc<Identity>,
    /// Who this agent trusts, re-read at every accept point so a revocation
    /// takes effect on the very next frame.
    pub pins: PinStore,
    /// This agent's own view of the fleet: the ONLY source of a forwarding
    /// address (rule R4).
    pub fleet: Arc<crate::fleet::LocalFleet>,
    /// What answers rank requests.
    pub rank: Arc<dyn RankService>,
    /// The control core a terminal `Control` executes through — the same one
    /// the browser session uses (rule T3).
    pub control: Arc<ControlHost>,
    /// The port peers listen on, attached to a resolved forwarding address.
    pub peer_port: u16,
    /// How long a forward waits for the target to execute and answer.
    /// Production wiring passes [`crate::peer::control::RELAY_ANSWER_BUDGET`];
    /// a field rather than the constant inlined so a test does not need a
    /// real minute to prove the timeout path.
    pub answer_budget: Duration,
}

/// Introduce ourselves and then answer this pinned peer until it hangs up.
///
/// One entry point for the daemon listener and the integration tests, so the
/// dispatch under test is the dispatch in production.
pub(crate) async fn serve_peer_connection<S>(stream: &mut S, ctx: &PeerServe, sender: NodeId)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let vitals = ctx.fleet.local_vitals_and_id().map(|(_, v)| v);
    // An unreadable pin store degrades to "did not say" (the intro's
    // `vouched` stays `None`), never to an affirmative "no pins" that would
    // retract every vouch this agent previously made.
    let mut intro = crate::peer::link::SelfIntro::new(ctx.fleet.can_launch(), "");
    if let Ok(digest) = crate::peer::link::fleet_digest(&ctx.pins, &ctx.fleet) {
        intro = intro.with_vouched(digest);
    }
    if crate::peer::link::serve_query(stream, &intro, vitals, &ctx.fleet.local_addresses())
        .await
        .is_err()
    {
        return;
    }
    serve_frames(stream, ctx, sender).await;
}

/// Answer a pinned peer's frames until it hangs up.
async fn serve_frames<S>(stream: &mut S, ctx: &PeerServe, sender: NodeId)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let local = ctx.identity.id();
    loop {
        let Ok(frame) = read_frame(stream).await else {
            return;
        };
        let reply = match frame {
            PeerFrame::PreviewRank { assignment } => match ctx.rank.render(&assignment) {
                Ok((command, unmapped)) => PeerFrame::RankPreviewed { command, unmapped },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            PeerFrame::Prepare { assignment, epoch } => PeerFrame::Prepared {
                reply: ctx.rank.prepare(&epoch, &assignment),
                epoch,
            },
            // Commit deliberately carries no assignment: what starts is what
            // this machine rendered and stored at prepare time, so a head
            // compromised between the phases cannot substitute anything.
            PeerFrame::Commit { epoch } => match ctx.rank.commit(&epoch) {
                Ok(container) => PeerFrame::Committed { epoch, container },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            PeerFrame::IsRankAlive { container } => PeerFrame::RankLiveness {
                // Unaskable is not alive: a rank whose state we cannot read
                // must not be counted as part of a whole cluster.
                running: ctx.rank.alive(&container).unwrap_or(false),
                container,
            },
            PeerFrame::StopRank { container } => {
                let _ = ctx.rank.stop(&container);
                PeerFrame::RankStopped { container }
            }
            // Abort is acknowledged rather than answered with a result: the
            // head is already rolling back, and a failure to release must not
            // mask whatever caused the rollback. Acknowledged whether or not
            // the container was there: a rollback asking twice, or asking
            // about a rank that never started, is an ordinary race and not
            // something the head can act on.
            PeerFrame::Abort { epoch } => {
                ctx.rank.abort(&epoch);
                PeerFrame::Aborted { epoch }
            }
            PeerFrame::Control { req } => PeerFrame::ControlReply {
                rep: terminal_control(&ctx.control.control(), sender, &ctx.pins, local, req),
            },
            PeerFrame::ControlTo { node, req } => PeerFrame::ControlReply {
                rep: relay_control(ctx, sender, node, req).await,
            },
            // Anything else is out of place mid-serving — a hello, a pairing
            // frame — and the conversation ends, exactly as before the
            // control frames existed.
            _ => return,
        };
        if write_frame(stream, &reply).await.is_err() {
            return;
        }
    }
}

/// Execute a terminal `Control` HERE (rules T1–T4).
///
/// T1 — `sender` was authenticated by the pinned-SPKI TLS gate before this
/// dispatch was reached. T4 is this signature: a control core, the sender,
/// the pin store, and this agent's own name — no dialer, no fleet, no
/// address book — so a forward cannot be written in here without widening
/// the signature in review, and the frame itself has no target field.
fn terminal_control(
    control: &LocalControl<'_>,
    sender: NodeId,
    pins: &PinStore,
    local: NodeId,
    req: ControlReq,
) -> ControlRep {
    // T2 — the grant, re-read NOW. "Pinned" is authentication; `controller`
    // is the authorization. Without this check the terminal frame would
    // silently widen what every existing pin means — a pin written before
    // control frames existed would become a license to stop the operator's
    // local workloads.
    if let Err(refusal) = grant_of(pins, sender, local) {
        return ControlRep::Refused {
            by: local,
            error: AgentError::ControlRefused {
                node: local,
                reason: refusal,
            },
        };
    }
    // T3 — through the identical core the local browser path uses: same
    // schema validation, same AlreadyRunning, same log-line cap. No argv, no
    // container id, no address was taken from the wire — `ControlReq` cannot
    // carry one.
    match control.execute(req) {
        Ok(rep) => rep,
        Err(error) => ControlRep::Refused { by: local, error },
    }
}

/// Forward one control request one hop (rules R1–R6).
///
/// R1 — `sender` was authenticated by the pinned-SPKI TLS gate. R6 — no code
/// path here emits `ControlTo`: the only outbound frame is the terminal
/// `Control` written by [`send_control`], which cannot name a further hop.
async fn relay_control(
    ctx: &PeerServe,
    sender: NodeId,
    node: NodeId,
    req: ControlReq,
) -> ControlRep {
    let local = ctx.identity.id();
    let refused = |detail: String| ControlRep::Refused {
        by: local,
        error: AgentError::RelayRefused { node, detail },
    };

    // R2 — the requester's grant, re-read NOW, checked here as well as at
    // the target: a relay that forwarded for any pinned peer would let an
    // ungranted machine spend the RELAY's grant at the target (the confused
    // deputy this rule exists to stop).
    if let Err(refusal) = grant_of(&ctx.pins, sender, local) {
        return refused(refusal);
    }

    // R3 — the target must be in THIS agent's own pin store: never a vouched
    // entry, never trusted from the frame. This is the one-hop knowledge
    // rule at the control layer — a relay cannot relay a relay.
    let Ok(pins) = ctx.pins.load() else {
        return refused("this agent's pin store could not be read".to_owned());
    };
    if !pins.contains_key(&node) {
        return refused(format!(
            "{} is not a peer of this agent; a relay only forwards to machines \
             it has itself pinned",
            node.short()
        ));
    }

    // R4 — the address comes from THIS agent's own reports and pin store.
    // The frame contributed zero bytes to the dial: it cannot even carry an
    // address, so this agent cannot be used as an address-scanning proxy.
    let Some(addr) = ctx
        .fleet
        .control_address(node)
        .and_then(|a| a.parse::<std::net::IpAddr>().ok())
        .map(|ip| std::net::SocketAddr::new(ip, ctx.peer_port))
    else {
        return refused(format!(
            "this agent has no known address for {}",
            node.short()
        ));
    };

    // R5 — dial through the shared choke point (SPKI re-verified against the
    // pin, identity re-derived), require the target's control support from
    // ITS hello, and re-issue `req` byte-identical in a terminal frame AS
    // OURSELVES. The target's answer is copied back verbatim.
    let intro = crate::peer::link::SelfIntro::new(ctx.fleet.can_launch(), "");
    match send_control(
        &ctx.identity,
        ctx.pins.clone(),
        addr,
        node,
        &intro,
        &req,
        ctx.answer_budget,
    )
    .await
    {
        Ok(rep) => rep,
        Err(e) => refused(format!("{e:#}")),
    }
}

/// The shared grant check behind R2 and T2: one meaning, both accept points.
///
/// Returns the operator-facing refusal on failure, naming the exact command
/// to run, and the machine to run it on, so the fix is copy-paste rather
/// than archaeology. An
/// unreadable pin store refuses (fail closed) — control must never be
/// granted by a disk error.
/// The refusal shown when a peer is pinned but not granted control.
///
/// Pure, so it can be asserted without a pin store — and so the wording is
/// decided in one place rather than inline in a match arm nobody re-reads.
///
/// It names the machine to run the command ON. The refusal is carried back to a
/// browser on someone else's laptop, where "this machine" reads as the laptop
/// they are looking at: the operator runs it on the wrong box, sees it succeed,
/// changes nothing relevant, and is refused again with no new information.
/// Every hop of this design is a different machine, so a remedy that does not
/// say which one is not a remedy.
pub(super) fn grant_refusal(sender: NodeId, local: NodeId) -> String {
    format!(
        "{sender} is paired with {local} but not granted control of it. Run \
         `atlasctl peer grant-control {sender}` ON {local} to allow it.",
        sender = sender.short(),
        local = local.short()
    )
}

fn grant_of(pins: &PinStore, sender: NodeId, local: NodeId) -> Result<(), String> {
    let loaded = pins
        .load()
        .map_err(|_| "this agent's pin store could not be read; refusing control".to_owned())?;
    match loaded.get(&sender) {
        Some(pin) if pin.controller => Ok(()),
        Some(_) => Err(grant_refusal(sender, local)),
        // Defense in depth: the TLS gate should make this unreachable, but a
        // pin removed between the handshake and this frame must refuse.
        None => Err(format!(
            "{} is not paired with this machine",
            sender.short()
        )),
    }
}
