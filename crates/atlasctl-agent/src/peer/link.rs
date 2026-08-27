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

/// What this agent says about itself when it introduces itself.
///
/// Exists so the outbound hello cannot be written by hand at each call site.
/// It was, and every one of them claimed `can_launch: true` — including the
/// control-only agents whose entire purpose is to be unable to run a model. A
/// value that must be constructed from the truth is harder to get wrong than a
/// literal that must be remembered.
#[derive(Debug, Clone, PartialEq)]
pub struct SelfIntro {
    /// This node's display name.
    pub name: String,
    /// Whether this node can run a model. Never assumed.
    pub can_launch: bool,
    /// This node's accelerator tag, empty when it has none to report.
    pub accelerator: String,
    /// This node's operating system, coarsely.
    pub os: String,
}

impl SelfIntro {
    /// Describe this node. The name is derived; the capability must be supplied.
    #[must_use]
    pub fn new(can_launch: bool, accelerator: &str) -> Self {
        Self {
            name: crate::discovery::local_display_name().as_str().to_owned(),
            can_launch,
            accelerator: accelerator.to_owned(),
            os: crate::discovery::local_os(),
        }
    }
}

/// What a peer said when it introduced itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    /// Its display name.
    pub name: String,
    /// Whether it can run a model.
    pub can_launch: bool,
    /// Its accelerator tag.
    pub accelerator: String,
    /// Its operating system, coarsely.
    pub os: String,
    /// The addresses it is reachable on, with subnets.
    pub addresses: Vec<atlasctl_protocol::fleet::NodeAddress>,
}

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
    /// Its operating system, coarsely.
    pub os: String,
    /// Its vitals, when it sent any.
    pub vitals: Option<NodeVitals>,
    /// The class of the link we actually reached it over.
    pub link: LinkClass,
    /// The addresses it says it is reachable on, with their subnets.
    ///
    /// Said over the authenticated channel by something holding the pinned
    /// key, so it can be believed — unlike a beacon, which reports whatever
    /// address the multicast arrived from and no subnet at all.
    pub addresses: Vec<atlasctl_protocol::fleet::NodeAddress>,
}

/// Dial a pinned peer and complete the mutually authenticated handshake.
///
/// Shared by every verb that reaches another machine, so there is exactly one
/// place that decides what "connected to the right peer" means. A second copy
/// of this logic is how one call site ends up skipping the identity re-check.
///
/// # Errors
/// If the peer cannot be reached, is not pinned, or is not the peer expected.
pub async fn dial(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    expect: NodeId,
) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
    let cfg = client_config(identity, PinnedPeerVerifier::pinned(pins, Some(expect)))?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));

    let tcp = tokio::time::timeout(DIAL_TIMEOUT, tokio::net::TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow::anyhow!("{addr} did not answer within {DIAL_TIMEOUT:?}"))?
        .with_context(|| format!("connecting to {addr}"))?;

    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")
        .context("building a server name")?
        .to_owned();
    let tls = tokio::time::timeout(DIAL_TIMEOUT, connector.connect(name, tcp))
        .await
        .map_err(|_| anyhow::anyhow!("TLS handshake with {addr} timed out"))?
        .context("TLS handshake")?;

    // Belt and braces: the verifier already refused anything unpinned or
    // unexpected, and this re-derives the identity from the certificate so a
    // later refactor cannot attribute an answer to the wrong machine.
    let peer_id = {
        let (_, conn) = tls.get_ref();
        let cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{addr} sent no certificate"))?;
        peer_identity(&cert)?.0
    };
    if peer_id != expect {
        bail!("reached {peer_id} at {addr}, expected {expect}");
    }
    Ok(tls)
}

/// Introduce ourselves on an established channel.
///
/// # Errors
/// If the peer does not introduce itself back, or speaks another version.
pub async fn exchange_hello<S>(
    tls: &mut S,
    addr: SocketAddr,
    intro: &SelfIntro,
    local: &[atlasctl_protocol::fleet::NodeAddress],
) -> Result<Hello>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    write_frame(
        tls,
        &PeerFrame::Hello {
            version: PEER_PROTOCOL_VERSION,
            name: intro.name.clone(),
            can_launch: intro.can_launch,
            accelerator: intro.accelerator.clone(),
            os: intro.os.clone(),
            addresses: local.to_vec(),
        },
    )
    .await?;

    let hello = tokio::time::timeout(DIAL_TIMEOUT, read_frame(tls))
        .await
        .map_err(|_| anyhow::anyhow!("{addr} did not introduce itself"))??;

    match hello {
        PeerFrame::Hello {
            version,
            name,
            can_launch,
            accelerator,
            os,
            addresses,
        } => {
            anyhow::ensure!(
                version == PEER_PROTOCOL_VERSION,
                "this node speaks peer protocol {PEER_PROTOCOL_VERSION}, {name} speaks {version}"
            );
            Ok(Hello {
                name,
                can_launch,
                accelerator,
                os,
                addresses,
            })
        }
        other => bail!("expected a hello from {addr}, got {other:?}"),
    }
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
    intro: &SelfIntro,
    local: &[atlasctl_protocol::fleet::NodeAddress],
) -> Result<PeerReport> {
    let mut tls = dial(identity, pins, addr, expect).await?;
    let hello = exchange_hello(&mut tls, addr, intro, local).await?;

    // Vitals are optional: an agent may be up and simply have nothing to say
    // about its hardware, which is not a failure.
    let vitals = match tokio::time::timeout(Duration::from_millis(500), read_frame(&mut tls)).await
    {
        Ok(Ok(PeerFrame::Vitals { vitals })) => Some(*vitals),
        _ => None,
    };

    Ok(PeerReport {
        node: expect,
        name: hello.name,
        can_launch: hello.can_launch,
        accelerator: hello.accelerator,
        os: hello.os,
        vitals,
        link,
        addresses: hello.addresses,
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
    os: &str,
    vitals: Option<NodeVitals>,
    local: &[atlasctl_protocol::fleet::NodeAddress],
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
            os: os.to_owned(),
            addresses: local.to_vec(),
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
