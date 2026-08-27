// SPDX-License-Identifier: AGPL-3.0-only

//! Asking another machine to execute — or forward — one control request.
//!
//! The client half of the control frames. Every helper here dials through
//! [`link::dial`], the single "connected to the right peer" choke point, and
//! refuses locally when the peer has not advertised `version_max >= 2` — a v2
//! frame sent at a v1 build is dropped by its decoder, which reads as the
//! network eating the request and sends the operator to restart the wrong
//! machine.

use super::link::{self, DIAL_TIMEOUT, SelfIntro, exchange_hello};
use super::wire::{PEER_PROTOCOL_MAX, PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use crate::identity::{Identity, PinStore};
use anyhow::{Result, bail};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::{ControlRep, ControlReq};
use std::net::SocketAddr;
use std::time::Duration;

/// How long a relay waits for the target to execute and answer. Covers a
/// pull-free docker launch; a longer launch fails VISIBLY and is recovered
/// with `Status`/`Stop` on the target (the launcher's own state is the SSOT
/// for what is running — no idempotency tokens).
pub const RELAY_ANSWER_BUDGET: Duration = Duration::from_secs(60);

/// How long an origin waits for a relayed answer. Defined as a sum so it is
/// structurally impossible for the origin to give up before the relay has
/// even timed out — the deadlock where each leg waits out the other.
pub const ORIGIN_ANSWER_BUDGET: Duration = RELAY_ANSWER_BUDGET
    .saturating_add(DIAL_TIMEOUT)
    .saturating_add(Duration::from_secs(10));

/// How many unsolicited frames to skip while waiting for the reply.
///
/// The `peer/cluster.rs` precedent: a peer may push vitals at any moment, and
/// reading the next frame as *the* answer would mistake one for the reply —
/// while an unbounded skip would let a peer that only ever sends vitals hold
/// the caller open forever.
const SKIP_BUDGET: usize = 8;

/// Dial `expect` and ask it to execute `req` on its own hardware.
///
/// Used by the origin for a directly-pinned target and by a relay for its
/// forwarding leg — the only two writers of the terminal `Control` frame.
/// `budget` bounds the wait for the answer, not the dial.
///
/// # Errors
/// If the peer cannot be reached, is not the one expected, has not advertised
/// control support, or does not answer within `budget`.
pub async fn send_control(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    intro: &SelfIntro,
    req: &ControlReq,
    budget: Duration,
) -> Result<ControlRep> {
    let mut tls = link::dial(identity, pins, addr, expect).await?;
    let hello = exchange_hello(&mut tls, addr, intro, &[]).await?;
    ensure_control_capable(&hello, expect)?;
    write_frame(&mut tls, &PeerFrame::Control { req: req.clone() }).await?;
    control_reply(&mut tls, addr, budget).await
}

/// Dial `relay` and ask it to forward `req` one hop to `target`.
///
/// Carries the target as a `NodeId` and nothing else: the relay resolves the
/// address from its own state, so this caller cannot use it as an
/// arbitrary-address proxy even by accident.
///
/// # Errors
/// If the relay cannot be reached, is not the one expected, has not
/// advertised control support, or no answer arrives within the origin budget.
pub async fn send_control_to(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    relay: NodeId,
    target: NodeId,
    intro: &SelfIntro,
    req: &ControlReq,
) -> Result<ControlRep> {
    let mut tls = link::dial(identity, pins, addr, relay).await?;
    let hello = exchange_hello(&mut tls, addr, intro, &[]).await?;
    ensure_control_capable(&hello, relay)?;
    write_frame(
        &mut tls,
        &PeerFrame::ControlTo {
            node: target,
            req: req.clone(),
        },
    )
    .await?;
    control_reply(&mut tls, addr, ORIGIN_ANSWER_BUDGET).await
}

/// Refuse, by name, a peer that cannot decode control frames.
///
/// Checked against the hello of THIS connection rather than a cached report,
/// so a peer downgraded since the last poll is caught before the frame is
/// written at it.
fn ensure_control_capable(hello: &link::Hello, peer: NodeId) -> Result<()> {
    // An old build that never said a maximum speaks exactly the version the
    // hello's equality check just accepted.
    let version_max = hello.version_max.unwrap_or(PEER_PROTOCOL_VERSION);
    if version_max < 2 {
        bail!(
            "{} ({}) speaks peer protocol {version_max} and cannot carry control \
             frames (this build speaks up to {PEER_PROTOCOL_MAX}); upgrade it",
            hello.name,
            peer.short()
        );
    }
    Ok(())
}

/// Read exactly one `ControlReply`, skipping interleaved vitals, within
/// `budget` overall.
async fn control_reply<S>(tls: &mut S, addr: SocketAddr, budget: Duration) -> Result<ControlRep>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let deadline = tokio::time::Instant::now() + budget;
    for _ in 0..SKIP_BUDGET {
        let frame = match tokio::time::timeout_at(deadline, read_frame(tls)).await {
            Ok(f) => f?,
            Err(_) => bail!("{addr} did not answer the control request within {budget:?}"),
        };
        match frame {
            PeerFrame::Vitals { .. } => continue,
            PeerFrame::ControlReply { rep } => return Ok(rep),
            // Anything else is a protocol error, not something to wait
            // through: a peer answering control with a rank frame is
            // confused, and reading on would turn that into a timeout.
            other => bail!("expected a control reply from {addr}, got {other:?}"),
        }
    }
    bail!("{addr} kept sending vitals instead of answering the control request")
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod control_tests;
