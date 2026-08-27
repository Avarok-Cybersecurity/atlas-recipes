// SPDX-License-Identifier: AGPL-3.0-only

//! Serving a pinned peer's frames, split from `daemon.rs` for size.
//!
//! The seam is the dispatch boundary: `daemon.rs` owns listeners, spawn
//! loops and lifecycle, and this file owns what a frame from an
//! already-authenticated pinned peer is answered with. The control-frame
//! handlers (the terminal `Control` executor and the `ControlTo` relay) will
//! land here; until they do, those frames get a typed refusal naming this
//! agent — never a silent close, which reads as a network failure and sends
//! the operator to restart the wrong machine.

use crate::peer::wire::{PeerFrame, read_frame, write_frame};
use crate::rank::RankService;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::{AgentError, ControlRep};
use std::sync::Arc;

/// Answer a pinned peer's rank requests until it hangs up.
///
/// `local` is this agent's own id, for naming WHO refused in a control
/// refusal — "dgx1 could not reach dgx3" and "dgx3 said no" send the operator
/// to different machines.
pub(super) async fn serve_rank_requests<S>(
    stream: &mut S,
    rank: &Arc<dyn RankService>,
    local: NodeId,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        let Ok(frame) = read_frame(stream).await else {
            return;
        };
        let reply = match frame {
            PeerFrame::PreviewRank { assignment } => match rank.render(&assignment) {
                Ok((command, unmapped)) => PeerFrame::RankPreviewed { command, unmapped },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            PeerFrame::Prepare { assignment, epoch } => PeerFrame::Prepared {
                reply: rank.prepare(&epoch, &assignment),
                epoch,
            },
            // Commit deliberately carries no assignment: what starts is what
            // this machine rendered and stored at prepare time, so a head
            // compromised between the phases cannot substitute anything.
            PeerFrame::Commit { epoch } => match rank.commit(&epoch) {
                Ok(container) => PeerFrame::Committed { epoch, container },
                Err(e) => PeerFrame::RankRefused {
                    reason: e.to_string(),
                },
            },
            PeerFrame::IsRankAlive { container } => PeerFrame::RankLiveness {
                // Unaskable is not alive: a rank whose state we cannot read
                // must not be counted as part of a whole cluster.
                running: rank.alive(&container).unwrap_or(false),
                container,
            },
            PeerFrame::StopRank { container } => {
                let _ = rank.stop(&container);
                PeerFrame::RankStopped { container }
            }
            // Abort is acknowledged rather than answered with a result: the
            // head is already rolling back, and a failure to release must not
            // mask whatever caused the rollback. Acknowledged whether or not
            // the container was there: a rollback asking twice, or asking
            // about a rank that never started, is an ordinary race and not
            // something the head can act on.
            PeerFrame::Abort { epoch } => {
                rank.abort(&epoch);
                PeerFrame::Aborted { epoch }
            }
            // The control surface is not served by this dispatch yet. The
            // frames exist on the wire ahead of their handlers, so a peer
            // that sends one gets a refusal that names this agent and says
            // what is missing — a silent drop here would be indistinguishable
            // from the network eating the request.
            PeerFrame::Control { .. } => PeerFrame::ControlReply {
                rep: ControlRep::Refused {
                    by: local,
                    error: AgentError::ControlRefused {
                        node: local,
                        reason: "this agent does not serve the control surface yet".to_owned(),
                    },
                },
            },
            PeerFrame::ControlTo { node, .. } => PeerFrame::ControlReply {
                rep: ControlRep::Refused {
                    by: local,
                    error: AgentError::RelayRefused {
                        node,
                        detail: "this agent does not forward control requests yet".to_owned(),
                    },
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{PrepareReply, RankAssignment};
    use anyhow::Result;

    /// A rank service that must never be reached: the frames under test are
    /// refused by dispatch, before any rank verb could run.
    struct Unreachable;

    impl RankService for Unreachable {
        fn render(&self, _: &RankAssignment) -> Result<(String, Vec<String>)> {
            panic!("a control frame must not reach the rank service");
        }
        fn content_hash(&self, _: &str) -> Result<String> {
            panic!("a control frame must not reach the rank service");
        }
        fn recipe_port(&self, _: &str) -> Result<Option<u16>> {
            panic!("a control frame must not reach the rank service");
        }
        fn prepare(&self, _: &str, _: &RankAssignment) -> PrepareReply {
            panic!("a control frame must not reach the rank service");
        }
        fn commit(&self, _: &str) -> Result<String> {
            panic!("a control frame must not reach the rank service");
        }
        fn alive(&self, _: &str) -> Result<bool> {
            panic!("a control frame must not reach the rank service");
        }
        fn stop(&self, _: &str) -> Result<()> {
            panic!("a control frame must not reach the rank service");
        }
        fn abort(&self, _: &str) {
            panic!("a control frame must not reach the rank service");
        }
    }

    /// Until the handlers land, a control frame must be answered with a typed
    /// refusal that names WHO refused — not dropped by the catch-all arm,
    /// which closes the connection and is indistinguishable from the network
    /// eating the request.
    #[tokio::test]
    async fn an_unserved_control_frame_is_refused_by_name_not_dropped() {
        let local = NodeId::from_bytes([1; 32]);
        let target = NodeId::from_bytes([7; 32]);
        let cases = [
            (
                PeerFrame::Control {
                    req: atlasctl_protocol::msg::ControlReq::Status,
                },
                "control",
            ),
            (
                PeerFrame::ControlTo {
                    node: target,
                    req: atlasctl_protocol::msg::ControlReq::Status,
                },
                "control_to",
            ),
        ];
        for (frame, what) in cases {
            let (mut ours, mut theirs) = tokio::io::duplex(1 << 16);
            let rank: Arc<dyn RankService> = Arc::new(Unreachable);
            let server =
                tokio::spawn(async move { serve_rank_requests(&mut theirs, &rank, local).await });

            write_frame(&mut ours, &frame).await.expect("send");
            let reply = read_frame(&mut ours)
                .await
                .unwrap_or_else(|e| panic!("{what} must be answered, not dropped: {e}"));
            match reply {
                PeerFrame::ControlReply {
                    rep: ControlRep::Refused { by, .. },
                } => {
                    assert_eq!(by, local, "the refusal must name who refused");
                }
                other => panic!("{what} expected a typed refusal, got {other:?}"),
            }
            drop(ours);
            server.await.expect("serving task ends cleanly");
        }
    }
}
