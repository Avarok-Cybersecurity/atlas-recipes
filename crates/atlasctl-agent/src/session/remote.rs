// SPDX-License-Identifier: AGPL-3.0-only

//! Routing a control verb toward another machine (rules O1–O5).
//!
//! Split from `session.rs` on the 500-line cap, along the trust seam: this
//! file decides WHERE a control verb goes and what the page is told about
//! the path; its parent decides everything about this machine. The session
//! stays transport-free — the dialling lives behind [`ControlRelay`], the
//! same shape as [`ClusterControl`](super::ClusterControl) — so every rule
//! here is testable with a fake relay and no sockets.

use super::{Session, err};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::{AgentError, ClientMsg, ControlRep, ControlReq, ServerMsg};

/// Routes a control request toward a non-local node.
///
/// A trait so the session stays transport-free, exactly like
/// [`ClusterControl`](super::ClusterControl) (same sync `fn` shape; the
/// driver blocks on the runtime as the cluster driver does). `None` in
/// [`SessionDeps::relay`](super::SessionDeps::relay) means this agent cannot
/// reach other machines, answered with a typed refusal rather than
/// pretended.
pub trait ControlRelay: Send + Sync {
    /// Returns the reply plus HOW it went: `via: None` = direct terminal
    /// dial, `Some(relay)` = forwarded. The session copies this into the
    /// reply's provenance fields verbatim — it records what was DONE, so the
    /// UI never has to guess the path.
    ///
    /// # Errors
    /// `NotRoutable` when this agent has no route at all, `RelayRefused` /
    /// `ControlRefused` when someone on the path said no.
    fn control(
        &self,
        target: NodeId,
        req: ControlReq,
    ) -> Result<(ControlRep, Option<NodeId>), AgentError>;
}

impl Session<'_> {
    /// Intercept a control verb aimed at another machine.
    ///
    /// `None` means the verb is local — `on: None`, or `on` naming this very
    /// machine (rule O1) — and the ordinary arms should run, unchanged.
    /// Anything else is answered here, so no arm below the dispatch can ever
    /// execute a remote-addressed verb on this box: that silent
    /// misattribution is the exact thing the provenance fields exist to
    /// prevent.
    pub(super) fn route_remote(&mut self, msg: &ClientMsg) -> Option<Vec<ServerMsg>> {
        let (id, target, req) = as_remote(msg)?;
        if self.is_local_node(target) {
            return None;
        }
        Some(vec![self.forward(id, target, req)])
    }

    /// Whether `node` is this machine, by the fleet's own account.
    ///
    /// A session without a fleet view cannot recognise its own id, so a
    /// `Some(target)` there is treated as remote and refused as unroutable —
    /// fail closed, never "probably meant here".
    fn is_local_node(&self, node: NodeId) -> bool {
        self.deps
            .fleet
            .is_some_and(|f| f.nodes().iter().any(|n| n.is_local && n.id == node))
    }

    /// Rules O2–O5 live behind the relay; this applies its verdict and
    /// states the provenance.
    fn forward(&mut self, id: u32, target: NodeId, req: ControlReq) -> ServerMsg {
        let Some(relay) = self.deps.relay else {
            return err(
                Some(id),
                AgentError::NotRoutable {
                    node: target,
                    reason: "this agent has no way to reach other machines".to_owned(),
                },
            );
        };
        match relay.control(target, req.clone()) {
            Ok((rep, via)) => rep_to_msg(id, target, via, &req, rep),
            Err(e) => err(Some(id), e),
        }
    }
}

/// The seven forwardable verbs, mapped onto the closed [`ControlReq`]
/// vocabulary. Everything else — including the pairing, join and cluster
/// verbs, which have no `on` field — returns `None` and stays local, so the
/// mapping cannot be widened without widening `ControlReq` itself.
fn as_remote(msg: &ClientMsg) -> Option<(u32, NodeId, ControlReq)> {
    match msg {
        ClientMsg::ListRecipes { id, on: Some(n) } => Some((*id, *n, ControlReq::ListRecipes)),
        ClientMsg::Preview {
            id,
            recipe,
            settings,
            on: Some(n),
        } => Some((
            *id,
            *n,
            ControlReq::Preview {
                recipe: recipe.clone(),
                settings: settings.clone(),
            },
        )),
        ClientMsg::Launch {
            id,
            recipe,
            settings,
            on: Some(n),
        } => Some((
            *id,
            *n,
            ControlReq::Launch {
                recipe: recipe.clone(),
                settings: settings.clone(),
            },
        )),
        ClientMsg::Stop {
            id,
            recipe,
            on: Some(n),
        } => Some((
            *id,
            *n,
            ControlReq::Stop {
                recipe: recipe.clone(),
            },
        )),
        ClientMsg::Status { id, on: Some(n) } => Some((*id, *n, ControlReq::Status)),
        ClientMsg::LaunchStats {
            id,
            recipe,
            on: Some(n),
        } => Some((
            *id,
            *n,
            ControlReq::Stats {
                recipe: recipe.clone(),
            },
        )),
        ClientMsg::LaunchLogs {
            id,
            recipe,
            lines,
            on: Some(n),
        } => Some((
            *id,
            *n,
            ControlReq::Logs {
                recipe: recipe.clone(),
                lines: *lines,
            },
        )),
        _ => None,
    }
}

/// Wrap the target's answer for the browser, stating provenance.
///
/// The answer must be the SHAPE the request asked for: a relay that answers
/// `Stop` with someone's `Recipes` is lying or confused, and rendering the
/// mismatch as if it were the reply would misattribute an answer to a verb
/// that never produced it. Refusals pass through typed, under the browser's
/// correlation id.
fn rep_to_msg(
    id: u32,
    target: NodeId,
    via: Option<NodeId>,
    asked: &ControlReq,
    rep: ControlRep,
) -> ServerMsg {
    let on = Some(target);
    match (asked, rep) {
        (ControlReq::ListRecipes, ControlRep::Recipes { recipes }) => ServerMsg::Recipes {
            id,
            recipes,
            on,
            via,
        },
        (ControlReq::Preview { .. }, ControlRep::Previewed { command, unapplied }) => {
            ServerMsg::Preview {
                id,
                command,
                unapplied,
                on,
                via,
            }
        }
        (
            ControlReq::Launch { .. },
            ControlRep::Started {
                recipe,
                container,
                endpoint,
            },
        ) => ServerMsg::Started {
            id,
            recipe,
            container,
            endpoint,
            on,
            via,
        },
        (ControlReq::Stop { .. }, ControlRep::Stopped { recipe }) => ServerMsg::Stopped {
            id,
            recipe,
            on,
            via,
        },
        (ControlReq::Status, ControlRep::Status { running }) => ServerMsg::Status {
            id,
            running,
            on,
            via,
        },
        (ControlReq::Stats { .. }, ControlRep::Stats { recipe, stats }) => ServerMsg::Stats {
            id,
            recipe,
            stats,
            on,
            via,
        },
        (
            ControlReq::Logs { .. },
            ControlRep::Logs {
                recipe,
                container,
                lines,
                running,
            },
        ) => ServerMsg::Logs {
            id,
            recipe,
            container,
            lines,
            running,
            on,
            via,
        },
        // `by` is dropped: the browser's `Error` frame has no field for it,
        // and giving it one would change a frame every call site builds. The
        // attribution it carried is not lost — a relay's own failure names
        // itself inside `RelayRefused.via`, and a target's refusal already
        // names itself in `ControlRefused.node`. What `by` alone could still
        // tell us is a relay refusing with some THIRD error kind, which no
        // path emits today; if one appears, that is the moment to widen the
        // frame rather than now.
        (_, ControlRep::Refused { by: _, error }) => err(Some(id), error),
        (_, other) => err(
            Some(id),
            AgentError::InvalidMessage {
                detail: format!(
                    "the answer relayed for {} did not match the request: {other:?}",
                    target.short()
                ),
            },
        ),
    }
}
