// SPDX-License-Identifier: AGPL-3.0-only

//! Dialling a machine and completing the pairing ceremony against it.
//!
//! Extracted so the two callers cannot drift. `atlasctl peer add` runs this
//! from a terminal; the browser channel runs the same function through a port,
//! because a pairing driven from a web page and a pairing driven from a shell
//! must be the *same* exchange — the moment they are two implementations, one
//! of them is the weaker one and nobody knows which.
//!
//! Only the initiator side lives here. The responder is
//! [`super::pair::run`] with [`Role::Responder`], reached through the peer
//! listener.
//!
//! What authenticates this is the **code**, not the certificate: the peer is by
//! definition not pinned yet, so [`PinnedPeerVerifier::pairing`] accepts an
//! unknown certificate and SPAKE2 decides whether the other end knew the
//! secret. The TLS exporter is mixed into the confirmation transcript, so a
//! relay terminating two sessions produces two different exporters and fails
//! key confirmation on both sides — which is the property that makes it safe to
//! do this without a human comparing anything.

use super::pair::{Paired, Role, run};
use super::tls::{PinnedPeerVerifier, client_config, peer_identity};
use crate::identity::{Identity, PinStore};
use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;

/// Dial a machine and pair with it, as the initiator.
///
/// Returns what was agreed. **Writes no pin** — recording trust is the
/// caller's decision, and separating them is what lets the browser show the
/// verification words before anything is trusted.
///
/// # Errors
/// If the machine cannot be reached, sends no certificate, or the ceremony
/// fails — which is what a wrong code and a relayed connection both look like.
pub async fn dial_and_pair(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    code: &str,
) -> Result<Paired> {
    let cfg = client_config(identity, PinnedPeerVerifier::pairing(pins))?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let tcp = tokio::net::TcpStream::connect(addr)
        .await
        .with_context(|| format!("connecting to {addr}"))?;
    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")
        .context("building a server name")?
        .to_owned();
    let mut tls = connector
        .connect(name, tcp)
        .await
        .context("TLS handshake")?;

    let (peer_id, binding) = {
        let (_, conn) = tls.get_ref();
        let cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the other machine sent no certificate"))?;
        let (id, _) = peer_identity(&cert)?;
        (id, crate::pairing::binding_from_client(conn)?)
    };

    run(&mut tls, Role::Initiator, identity, peer_id, code, binding).await
}
