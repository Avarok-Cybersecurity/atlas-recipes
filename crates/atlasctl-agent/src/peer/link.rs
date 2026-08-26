// SPDX-License-Identifier: AGPL-3.0-only

//! Staying connected to the machines you have paired with.
//!
//! Pairing establishes trust; this keeps it useful. A [`PeerLink`] dials a
//! pinned peer over the authenticated channel and exchanges what a beacon
//! cannot be trusted to carry: the peer's real link class, its vitals, and
//! whether it is actually up.
//!
//! That last point is the important one. Liveness derived from beacons is
//! wrong in both directions — a machine can be broadcasting while its agent is
//! wedged, and a perfectly healthy machine can go quiet because a switch
//! filtered multicast. A peer is reachable when we can complete a mutually
//! authenticated handshake with it, and not otherwise.
//!
//! Failure here is ordinary. A peer that is switched off is the normal state of
//! a fleet, so a failed dial is recorded and retried with backoff rather than
//! logged as an error.

use super::tls::{PinnedPeerVerifier, client_config, peer_identity};
use super::wire::{PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use crate::identity::{Identity, PinStore};
use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::{LinkClass, NodeId, NodeVitals};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// How long to wait for a peer to answer before treating it as down.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

/// What a peer told us about itself over the authenticated channel.
///
/// Distinct from a beacon: everything here was said by something holding the
/// private key we pinned, so it can be believed.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerReport {
    /// Which peer.
    pub node: NodeId,
    /// Its display name, as it describes itself.
    pub name: String,
    /// Whether it can run a model.
    pub can_launch: bool,
    /// Its accelerator tag.
    pub accelerator: String,
    /// Its vitals, when it sent any.
    pub vitals: Option<NodeVitals>,
    /// The class of the link we actually reached it over.
    pub link: LinkClass,
}

/// Dial a pinned peer and ask how it is.
///
/// `expect` is the peer we intend to reach. Passing it means connecting to some
/// *other* agent — through a stale address, a DNS trick, or an ARP one — is an
/// error rather than a silent success.
///
/// `link` is how *we* classify the interface this address sits on, taken from
/// our own fabric probe. It is not asked of the peer, because a peer's opinion
/// of its own link is not evidence about the path between us.
///
/// # Errors
/// If the peer cannot be reached, is not the peer we expected, is not pinned,
/// or speaks a different protocol version.
pub async fn query(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
    link: LinkClass,
) -> Result<PeerReport> {
    let cfg = client_config(identity, PinnedPeerVerifier::pinned(pins, Some(expect)))?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));

    let tcp = tokio::time::timeout(DIAL_TIMEOUT, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow::anyhow!("{addr} did not answer within {DIAL_TIMEOUT:?}"))?
        .with_context(|| format!("connecting to {addr}"))?;

    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")
        .context("building a server name")?
        .to_owned();
    let mut tls = tokio::time::timeout(DIAL_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| anyhow::anyhow!("TLS handshake with {addr} timed out"))?
        .context("TLS handshake")?;

    // Belt and braces: the verifier already refused anything unpinned or
    // unexpected, and this re-derives the identity from the certificate so the
    // report cannot be attributed to the wrong machine by a later refactor.
    let (peer_id, _) = {
        let (_, conn) = tls.get_ref();
        let cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{addr} sent no certificate"))?;
        peer_identity(&cert)?
    };
    if peer_id != expect {
        bail!("reached {peer_id} at {addr}, expected {expect}");
    }

    write_frame(
        &mut tls,
        &PeerFrame::Hello {
            version: PEER_PROTOCOL_VERSION,
            name: crate::discovery::local_display_name().as_str().to_owned(),
            can_launch: true,
            accelerator: String::new(),
        },
    )
    .await?;

    let hello = tokio::time::timeout(DIAL_TIMEOUT, read_frame(&mut tls))
        .await
        .map_err(|_| anyhow::anyhow!("{addr} did not introduce itself"))??;

    let (name, can_launch, accelerator) = match hello {
        PeerFrame::Hello {
            version,
            name,
            can_launch,
            accelerator,
        } => {
            anyhow::ensure!(
                version == PEER_PROTOCOL_VERSION,
                "this node speaks peer protocol {PEER_PROTOCOL_VERSION}, {name} speaks {version}"
            );
            (name, can_launch, accelerator)
        }
        other => bail!("expected a hello from {addr}, got {other:?}"),
    };

    // Vitals are optional: an agent may be up and simply have nothing to say
    // about its hardware, which is not a failure.
    let vitals = match tokio::time::timeout(Duration::from_millis(500), read_frame(&mut tls)).await
    {
        Ok(Ok(PeerFrame::Vitals { vitals })) => Some(*vitals),
        _ => None,
    };

    Ok(PeerReport {
        node: peer_id,
        name,
        can_launch,
        accelerator,
        vitals,
        link,
    })
}

/// Answer a peer that dialled us.
///
/// The mirror of [`query`]: introduce ourselves and offer a vitals sample. Only
/// reached after the TLS verifier has confirmed the caller is pinned, so there
/// is no authorization decision left to make here.
///
/// # Errors
/// If the connection fails mid-exchange.
pub async fn serve_query<S>(
    stream: &mut S,
    name: &str,
    can_launch: bool,
    accelerator: &str,
    vitals: Option<NodeVitals>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let hello = read_frame(stream).await?;
    anyhow::ensure!(
        matches!(hello, PeerFrame::Hello { .. }),
        "a peer must introduce itself first, got {hello:?}"
    );

    write_frame(
        stream,
        &PeerFrame::Hello {
            version: PEER_PROTOCOL_VERSION,
            name: name.to_owned(),
            can_launch,
            accelerator: accelerator.to_owned(),
        },
    )
    .await?;

    if let Some(v) = vitals {
        write_frame(
            stream,
            &PeerFrame::Vitals {
                vitals: Box::new(v),
            },
        )
        .await?;
    }
    Ok(())
}
