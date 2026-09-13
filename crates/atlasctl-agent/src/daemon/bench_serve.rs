// SPDX-License-Identifier: AGPL-3.0-only

//! Serving the bench frames: the grant check, the request dispatch, and the
//! attached event stream.
//!
//! The same discipline as `terminal_control`: the sender was authenticated by
//! the pinned-TLS gate; the `bench` grant is re-read from the pin store on
//! EVERY frame so a revocation lands on the next request; and nothing from
//! the wire reaches a process — the host renders the argv from its own
//! configuration.

use super::peer_serve::PeerServe;
use crate::bench::BenchHost;
use crate::identity::PinStore;
use crate::peer::bench::HEARTBEAT;
use crate::peer::wire::{PeerFrame, read_frame, write_frame};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::{BenchRefusal, BenchRep, BenchReq, JobId};
use atlasctl_protocol::msg::bench_event::{BenchEvent, EventKind};
use std::sync::Arc;
use std::time::Duration;

/// How long a write to an attached client may block before the client is
/// judged gone. A reader that does not drain is disconnected and resumes
/// from `from_seq` when it can.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// The remedy, naming the machine it must be run on.
pub fn bench_grant_refusal(sender: NodeId, local: NodeId) -> BenchRefusal {
    BenchRefusal::NotGranted {
        command: format!(
            "atlasctl peer grant-bench {sender} ON {local}",
            sender = sender.short(),
            local = local.short()
        ),
    }
}

fn grant_of(pins: &PinStore, sender: NodeId, local: NodeId) -> Result<(), BenchRefusal> {
    let loaded = pins.load().map_err(|_| BenchRefusal::NotConfigured {
        what: "this agent's pin store could not be read".into(),
        fix: "check the config directory's permissions on the node".into(),
    })?;
    match loaded.get(&sender) {
        Some(pin) if pin.bench => Ok(()),
        Some(_) => Err(bench_grant_refusal(sender, local)),
        None => Err(BenchRefusal::NotConfigured {
            what: format!("{} is not paired with this machine", sender.short()),
            fix: "pair first (`atlasctl agent pair` on the node, `atlasctl peer add` here)".into(),
        }),
    }
}

/// Answer one non-attach bench request HERE.
pub(crate) fn terminal_bench(
    host: Option<&Arc<BenchHost>>,
    sender: NodeId,
    pins: &PinStore,
    local: NodeId,
    local_name: atlasctl_protocol::fleet::DisplayName,
    disabled_reason: Option<&str>,
    req: BenchReq,
) -> BenchRep {
    if let Err(refusal) = grant_of(pins, sender, local) {
        return BenchRep::Refused { by: local, refusal };
    }
    let Some(host) = host else {
        // NodeInfo still answers, saying why the rest cannot.
        return match req {
            BenchReq::NodeInfo => BenchRep::NodeInfo {
                info: Box::new(crate::bench::nodeinfo::disabled(
                    local,
                    local_name,
                    disabled_reason.unwrap_or("bench is not configured on this node"),
                )),
            },
            _ => BenchRep::Refused {
                by: local,
                refusal: BenchRefusal::NotConfigured {
                    what: disabled_reason.unwrap_or("no bench.yaml").into(),
                    fix: "write <config-dir>/bench.yaml on the node and restart its agent".into(),
                },
            },
        };
    };
    match req {
        BenchReq::NodeInfo => BenchRep::NodeInfo {
            info: Box::new(crate::bench::nodeinfo::node_info(host)),
        },
        BenchReq::Submit {
            job_key,
            sha,
            gate,
            params,
            checkpoint,
            hardware,
            max_run_s,
            note,
        } => host.submit(
            sender, job_key, sha, gate, params, checkpoint, hardware, max_run_s, note,
        ),
        BenchReq::Status { job } => host.status(job),
        BenchReq::Cancel { job } => host.cancel(job),
        BenchReq::ArtifactList { job } => host.artifacts(job),
        BenchReq::ArtifactChunk {
            job,
            name,
            offset,
            len,
        } => host.chunk(job, name, offset, len),
        BenchReq::Attach { .. } => BenchRep::Refused {
            by: local,
            refusal: BenchRefusal::NotConfigured {
                what: "attach is a stream, not a request".into(),
                fix: "internal: the serving loop routes Attach before this point".into(),
            },
        },
    }
}

/// Turn `stream` into an event stream for `job`: replay from `from_seq`,
/// then tail, heartbeating, until `Done` is written or the client goes.
pub(crate) async fn serve_attach<S>(
    stream: &mut S,
    ctx: &PeerServe,
    sender: NodeId,
    job: JobId,
    from_seq: u64,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let local = ctx.identity.id();
    let refuse = |refusal: BenchRefusal| PeerFrame::BenchReply {
        rep: BenchRep::Refused { by: local, refusal },
    };
    if let Err(refusal) = grant_of(&ctx.pins, sender, local) {
        let _ = write_frame(stream, &refuse(refusal)).await;
        return;
    }
    let Some(host) = ctx.bench.as_ref() else {
        let _ = write_frame(
            stream,
            &refuse(BenchRefusal::NotConfigured {
                what: "no bench.yaml".into(),
                fix: "configure bench on the node".into(),
            }),
        )
        .await;
        return;
    };
    if host.store.load(&job).is_err() {
        let _ = write_frame(stream, &refuse(BenchRefusal::UnknownJob { job })).await;
        return;
    }
    let Ok(journal) = host.journal(&job) else {
        let _ = write_frame(stream, &refuse(BenchRefusal::UnknownJob { job })).await;
        return;
    };
    let mut watch = journal.watch();
    let mut next = from_seq.max(1);
    loop {
        // Everything journaled since `next`.
        let events = match journal.replay(next) {
            Ok(e) => e,
            Err(_) => return,
        };
        for e in events {
            let done = matches!(e.kind, EventKind::Done { .. });
            next = e.seq + 1;
            if tokio::time::timeout(
                WRITE_TIMEOUT,
                write_frame(stream, &PeerFrame::BenchEvent { event: e }),
            )
            .await
            .is_err()
            {
                return;
            }
            if done {
                let _ = tokio::io::AsyncWriteExt::shutdown(stream).await;
                return;
            }
        }
        // Nothing new: wait for the journal, a heartbeat tick, or the client
        // hanging up (a read that ends).
        let state = host
            .store
            .load(&job)
            .map_or(atlasctl_protocol::msg::bench::JobState::Done, |r| r.state);
        let heartbeat = BenchEvent {
            job: job.clone(),
            seq: journal.seq_high(),
            at_ms: crate::bench::job::now_s() * 1000,
            kind: EventKind::Heartbeat {
                state,
                seq_high: journal.seq_high(),
            },
        };
        if state.is_terminal() && journal.seq_high() < next {
            // Terminal with nothing left to send (the client attached past
            // the end). One heartbeat carries that fact — terminal state,
            // `seq_high` below what it asked for — so the client can end
            // its stream deliberately rather than on a dropped socket.
            let _ = tokio::time::timeout(
                WRITE_TIMEOUT,
                write_frame(stream, &PeerFrame::BenchEvent { event: heartbeat }),
            )
            .await;
            let _ = tokio::io::AsyncWriteExt::shutdown(stream).await;
            return;
        }
        tokio::select! {
            changed = watch.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            () = tokio::time::sleep(HEARTBEAT) => {
                if tokio::time::timeout(WRITE_TIMEOUT, write_frame(stream, &PeerFrame::BenchEvent { event: heartbeat }))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            frame = read_frame(stream) => {
                // The client may only hang up; any frame ends the stream.
                let _ = frame;
                return;
            }
        }
    }
}
