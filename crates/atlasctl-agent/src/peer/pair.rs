// SPDX-License-Identifier: AGPL-3.0-only

//! Driving the pairing ceremony across a real connection.
//!
//! [`crate::pairing`] owns the cryptography; this owns the conversation. The
//! split matters because the ordering is where these things go wrong, and the
//! ordering here is deliberate:
//!
//! 1. TLS completes first, with the verifier in its `pairing` mode — the peer
//!    is by definition not pinned yet, so the certificate check cannot be what
//!    authenticates it.
//! 2. Both sides export channel-binding material from **their own** TLS
//!    session.
//! 3. SPAKE2 messages are exchanged and each side derives a key.
//! 4. Confirmation MACs, computed over the key *and the binding*, are
//!    exchanged and checked in constant time.
//! 5. Only then is a pin written — and only after the caller has shown a human
//!    the verification words.
//!
//! Step 4 is what defeats a machine-in-the-middle. An attacker who terminates
//! two TLS sessions and relays between them holds two different exporters, so
//! the MACs disagree and both sides refuse. Without it, SPAKE2 would happily
//! authenticate both endpoints to an attacker sitting between two perfectly
//! authenticated connections.

use super::wire::{PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use crate::identity::Identity;
use crate::pairing::{Exchange, looks_like_code};
use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::NodeId;
use tokio::io::{AsyncRead, AsyncWrite};

/// What a completed ceremony produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    /// The peer's identity.
    pub node: NodeId,
    /// Its public key, hex encoded, ready to pin.
    pub public_key: String,
    /// Its display name.
    pub name: String,
    /// Words both humans compare before the pin is kept.
    pub verification: String,
}

/// Which side of the exchange this is.
///
/// The cryptography is symmetric; only who speaks first differs, and that has
/// to be decided somewhere rather than left to a race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Dialled the peer, and speaks first.
    Initiator,
    /// Accepted the connection, and answers.
    Responder,
}

/// Run the ceremony to completion over an established, authenticated-by-nothing
/// TLS stream.
///
/// `binding` must be the exporter material from *this* side's TLS session.
/// `peer_id` is who the transport believes it is talking to, taken from the
/// certificate; the ceremony proves whether that belief is worth anything.
///
/// # Errors
/// If the code is malformed, the peer speaks a different protocol version, the
/// exchange fails, or key confirmation does not match — which is what both a
/// wrong code and a relayed connection look like.
pub async fn run<S>(
    stream: &mut S,
    role: Role,
    identity: &Identity,
    peer_id: NodeId,
    code: &str,
    binding: Vec<u8>,
) -> Result<Paired>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    anyhow::ensure!(looks_like_code(code), "a pairing code is 8 digits");

    // --- hello ------------------------------------------------------------
    let me = PeerFrame::Hello {
        version: PEER_PROTOCOL_VERSION,
        name: crate::discovery::local_display_name().as_str().to_owned(),
        can_launch: true,
        accelerator: String::new(),
        // Pairing is about identity, not topology; addresses are exchanged on
        // the authenticated channel once there is a pin to believe them under.
        addresses: Vec::new(),
    };
    let peer_hello = match role {
        Role::Initiator => {
            write_frame(stream, &me).await?;
            read_frame(stream).await?
        }
        Role::Responder => {
            let h = read_frame(stream).await?;
            write_frame(stream, &me).await?;
            h
        }
    };
    let peer_name = match peer_hello {
        PeerFrame::Hello { version, name, .. } => {
            anyhow::ensure!(
                version == PEER_PROTOCOL_VERSION,
                "this node speaks peer protocol {PEER_PROTOCOL_VERSION}, the other speaks {version}"
            );
            name
        }
        other => bail!("expected a hello, got {other:?}"),
    };

    // --- SPAKE2 -----------------------------------------------------------
    let started = Exchange::start(code, identity.id(), peer_id, binding);
    let mine = PeerFrame::PairStart {
        message: hex::encode(&started.message),
    };
    let theirs = match role {
        Role::Initiator => {
            write_frame(stream, &mine).await?;
            read_frame(stream).await?
        }
        Role::Responder => {
            let t = read_frame(stream).await?;
            write_frame(
                stream,
                &PeerFrame::PairAnswer {
                    message: hex::encode(&started.message),
                },
            )
            .await?;
            t
        }
    };
    let peer_message = match theirs {
        PeerFrame::PairStart { message } | PeerFrame::PairAnswer { message } => {
            hex::decode(&message).context("peer sent a malformed pairing message")?
        }
        PeerFrame::PairRefused { reason } => bail!("the other node refused: {reason}"),
        other => bail!("expected a pairing message, got {other:?}"),
    };

    let confirmation = started.exchange.finish(&peer_message)?;

    // --- key confirmation -------------------------------------------------
    // A wrong code produces a different key here, not an error earlier, which
    // is deliberate: an early failure would say whether the code was right
    // before confirmation, and that is the oracle SPAKE2 exists to remove.
    let my_mac = PeerFrame::PairConfirm {
        mac: hex::encode(confirmation.mine()),
    };
    let their_frame = match role {
        Role::Initiator => {
            write_frame(stream, &my_mac).await?;
            read_frame(stream).await?
        }
        Role::Responder => {
            let t = read_frame(stream).await?;
            write_frame(stream, &my_mac).await?;
            t
        }
    };
    let their_mac = match their_frame {
        PeerFrame::PairConfirm { mac } => hex::decode(&mac).unwrap_or_default(),
        PeerFrame::PairRefused { reason } => bail!("the other node refused: {reason}"),
        other => bail!("expected key confirmation, got {other:?}"),
    };

    if let Err(e) = confirmation.verify(&their_mac) {
        // Tell the peer, so it stops waiting and shows the same reason rather
        // than a timeout.
        let _ = write_frame(
            stream,
            &PeerFrame::PairRefused {
                reason: "key confirmation failed".to_owned(),
            },
        )
        .await;
        return Err(e);
    }

    // --- exchange keys ----------------------------------------------------
    let accept = PeerFrame::PairAccepted {
        public_key: hex::encode(identity.public().as_bytes()),
    };
    let peer_accept = match role {
        Role::Initiator => {
            write_frame(stream, &accept).await?;
            read_frame(stream).await?
        }
        Role::Responder => {
            let t = read_frame(stream).await?;
            write_frame(stream, &accept).await?;
            t
        }
    };
    let public_key = match peer_accept {
        PeerFrame::PairAccepted { public_key } => public_key,
        PeerFrame::PairRefused { reason } => bail!("the other node refused: {reason}"),
        other => bail!("expected an acceptance, got {other:?}"),
    };

    // The key must be the one the transport already saw in the certificate.
    // Without this a peer could authenticate with one key and ask to be pinned
    // under another.
    crate::identity::verify_key_matches(peer_id, &public_key)
        .context("the key offered for pinning is not the key that authenticated")?;

    Ok(Paired {
        node: peer_id,
        public_key,
        name: peer_name,
        verification: confirmation.verification_words(),
    })
}
