// SPDX-License-Identifier: AGPL-3.0-only

//! Asking another machine to be a rank.
//!
//! Every verb here dials, introduces itself, asks one question and hangs up.
//! There is no long-lived cluster session, deliberately: a session would have
//! to survive a switch reboot and a laptop lid closing to be worth anything,
//! and a head that reconnects per phase discovers a dead rank at the phase
//! boundary — which is exactly where the rollback path already is.
//!
//! Vitals are unsolicited, so a peer can offer one at any moment. Every reader
//! here skips them rather than mistaking one for an answer, bounded so that a
//! peer sending nothing else cannot hold a phase open.

use super::link::{DIAL_TIMEOUT, dial, exchange_hello};
use super::wire::{PeerFrame, read_frame, write_frame};
use crate::cluster::{PrepareReply, RankAssignment};
use crate::identity::{Identity, PinStore};
use anyhow::{Result, bail};
use atlasctl_protocol::fleet::NodeId;
use std::net::SocketAddr;

/// How many unsolicited frames to skip before giving up on an answer.
const SKIP_BUDGET: usize = 8;

/// Read frames until one of them answers the question, skipping vitals.
///
/// `want` maps a frame to an answer, or `None` if it was not one. Anything that
/// is neither vitals nor an answer is a protocol error rather than something to
/// wait through — a peer that replies to prepare with a preview is confused,
/// and continuing to read would turn that into a timeout instead of a message.
async fn answer<S, T>(
    tls: &mut S,
    addr: SocketAddr,
    what: &str,
    mut want: impl FnMut(PeerFrame) -> Result<Option<T>>,
) -> Result<T>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    for _ in 0..SKIP_BUDGET {
        let frame = match tokio::time::timeout(DIAL_TIMEOUT, read_frame(tls)).await {
            Ok(f) => f?,
            Err(_) => bail!("{addr} did not answer the {what} in time"),
        };
        if matches!(frame, PeerFrame::Vitals { .. }) {
            continue;
        }
        if let Some(v) = want(frame)? {
            return Ok(v);
        }
    }
    bail!("{addr} kept sending vitals instead of answering the {what}")
}

/// Ask a rank to render the command it would run, without running it.
///
/// The head does not render this itself, and that is the point: it does not
/// know what recipe revision, hardware or flag table the other machine has, so
/// a preview it invented would be a guess presented as the thing that will
/// execute.
///
/// # Errors
/// If the peer cannot be reached, refuses the assignment, or answers with
/// something other than a preview.
pub async fn preview_rank(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    assignment: RankAssignment,
) -> Result<(String, Vec<String>)> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    exchange_hello(&mut tls, addr).await?;
    write_frame(
        &mut tls,
        &PeerFrame::PreviewRank {
            assignment: Box::new(assignment),
        },
    )
    .await?;

    answer(&mut tls, addr, "preview", |f| match f {
        PeerFrame::RankPreviewed { command, unmapped } => Ok(Some((command, unmapped))),
        PeerFrame::RankRefused { reason } => bail!("{reason}"),
        other => bail!("expected a rank preview, got {other:?}"),
    })
    .await
}

/// Ask a rank to validate and reserve. Nothing starts.
///
/// A refusal is an ordinary outcome and is returned as a value rather than an
/// error, because the head must collect every rank's answer before deciding —
/// treating the first refusal as an error would abandon ranks still holding
/// reservations.
///
/// # Errors
/// If the peer cannot be reached or answers with something other than a
/// prepare result. A *refusal* is not an error.
pub async fn prepare_rank(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    epoch: &str,
    assignment: RankAssignment,
) -> Result<PrepareReply> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    exchange_hello(&mut tls, addr).await?;
    write_frame(
        &mut tls,
        &PeerFrame::Prepare {
            assignment: Box::new(assignment),
            epoch: epoch.to_owned(),
        },
    )
    .await?;

    let want_epoch = epoch.to_owned();
    answer(&mut tls, addr, "prepare", move |f| match f {
        // A reply for another epoch is a stale answer from an earlier attempt,
        // not this one's; accepting it would let a previous cluster's yes
        // authorize this cluster's commit.
        PeerFrame::Prepared { epoch, reply } if epoch == want_epoch => Ok(Some(reply)),
        PeerFrame::Prepared { .. } => Ok(None),
        PeerFrame::RankRefused { reason } => Ok(Some(PrepareReply::Refused { reason })),
        other => bail!("expected a prepare result, got {other:?}"),
    })
    .await
}

/// Start what a rank prepared under this epoch.
///
/// Carries no assignment. What starts is what that machine rendered and stored
/// at prepare time, so a head compromised between the phases can start the
/// launch the operator already previewed, or not start it, and nothing else.
///
/// # Errors
/// If the peer cannot be reached, holds no such reservation, or the container
/// runtime refuses.
pub async fn commit_rank(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    epoch: &str,
) -> Result<String> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    exchange_hello(&mut tls, addr).await?;
    write_frame(
        &mut tls,
        &PeerFrame::Commit {
            epoch: epoch.to_owned(),
        },
    )
    .await?;

    let want_epoch = epoch.to_owned();
    answer(&mut tls, addr, "commit", move |f| match f {
        PeerFrame::Committed { epoch, container } if epoch == want_epoch => Ok(Some(container)),
        PeerFrame::Committed { .. } => Ok(None),
        PeerFrame::RankRefused { reason } => bail!("{reason}"),
        other => bail!("expected a commit result, got {other:?}"),
    })
    .await
}

/// Release a rank's reservation without starting anything.
///
/// Returns nothing, and callers ignore its failures on purpose: this runs when
/// something has already gone wrong, and a second failure must not replace the
/// reason the operator actually needs to read. A reservation left behind is
/// released by that machine's next prepare regardless.
///
/// # Errors
/// If the peer cannot be reached.
pub async fn abort_rank(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    epoch: &str,
) -> Result<()> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    exchange_hello(&mut tls, addr).await?;
    write_frame(
        &mut tls,
        &PeerFrame::Abort {
            epoch: epoch.to_owned(),
        },
    )
    .await?;
    let want_epoch = epoch.to_owned();
    answer(&mut tls, addr, "abort", move |f| match f {
        PeerFrame::Aborted { epoch } if epoch == want_epoch => Ok(Some(())),
        PeerFrame::Aborted { .. } => Ok(None),
        other => bail!("expected an abort acknowledgement, got {other:?}"),
    })
    .await
}

/// Stop a container a rank started.
///
/// Used by rollback, so failures are the caller's to ignore: this runs when
/// something has already gone wrong and the operator needs the original reason,
/// not this one.
///
/// # Errors
/// If the peer cannot be reached or answers with something else.
pub async fn stop_rank(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    container: &str,
) -> Result<()> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    exchange_hello(&mut tls, addr).await?;
    write_frame(
        &mut tls,
        &PeerFrame::StopRank {
            container: container.to_owned(),
        },
    )
    .await?;
    let want = container.to_owned();
    answer(&mut tls, addr, "stop", move |f| match f {
        PeerFrame::RankStopped { container } if container == want => Ok(Some(())),
        PeerFrame::RankStopped { .. } => Ok(None),
        other => bail!("expected a stop acknowledgement, got {other:?}"),
    })
    .await
}
