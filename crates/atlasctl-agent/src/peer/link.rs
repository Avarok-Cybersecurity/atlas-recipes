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
use super::wire::{PEER_PROTOCOL_MAX, PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use crate::identity::{Identity, PinStore};
use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::{LinkClass, MAX_VOUCHED, NodeId, NodeVitals, VouchedPeer};
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
    /// Highest peer-protocol version this build speaks. A field rather than
    /// the constant inlined at the write site, so a test can introduce an
    /// old build; [`Self::new`] always states this build's real maximum.
    pub version_max: Option<u32>,
    /// The fleet digest to announce, when the caller has one. `None` on the
    /// per-verb cluster dials, the same way they already pass no addresses:
    /// "did not say" is honest there, while `Some(vec![])` would affirm
    /// having no pins.
    pub vouched: Option<Vec<VouchedPeer>>,
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
            version_max: Some(PEER_PROTOCOL_MAX),
            vouched: None,
        }
    }

    /// Attach a fleet digest to this introduction.
    #[must_use]
    pub fn with_vouched(mut self, digest: Vec<VouchedPeer>) -> Self {
        self.vouched = Some(digest);
        self
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
    /// Highest peer-protocol version it can speak. `None` = a build that
    /// predates the field, i.e. exactly the `version` it sent.
    pub version_max: Option<u32>,
    /// The peers it has itself pinned, as it stated them. `None` = did not
    /// say; `Some(vec![])` = affirmatively no pins. Kept distinct so an old
    /// build's silence is never read as a retraction.
    pub vouched: Option<Vec<VouchedPeer>>,
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
    /// The fleet digest it sent, when it sent one. Second-hand knowledge:
    /// recorded in the vouch table, never in this agent's own digest.
    pub vouched: Option<Vec<VouchedPeer>>,
    /// Highest peer-protocol version it can speak, normalized: an old build
    /// that never said is exactly the `version` it did say. Control frames
    /// are refused locally toward any peer whose value here is below 2,
    /// rather than sent at a build that would drop the connection.
    pub peer_version_max: u32,
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
            version_max: intro.version_max,
            vouched: intro.vouched.clone(),
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
            version_max,
            vouched,
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
                version_max,
                vouched,
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
        vouched: hello.vouched,
        // Normalized here so nothing downstream re-derives it differently:
        // a build that never said a maximum speaks exactly the version the
        // equality check just accepted.
        peer_version_max: hello.version_max.unwrap_or(PEER_PROTOCOL_VERSION),
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
    intro: &SelfIntro,
    vitals: Option<NodeVitals>,
    local: &[atlasctl_protocol::fleet::NodeAddress],
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let hello = read_frame(stream).await?;
    // Both halves of the exact-match rule, on both sides. The outbound
    // `exchange_hello` has always checked the version; this side only checked
    // that the frame WAS a hello, so the invariant "exact match on the peer
    // channel" held in one direction. Not exploitable — the caller is already
    // through the pinned-SPKI TLS gate — but an invariant that is true only
    // outbound is one nobody can rely on when the version next moves.
    match &hello {
        PeerFrame::Hello { version, .. } => anyhow::ensure!(
            *version == PEER_PROTOCOL_VERSION,
            "this node speaks peer protocol {PEER_PROTOCOL_VERSION}, the caller speaks {version}"
        ),
        other => anyhow::bail!("a peer must introduce itself first, got {other:?}"),
    }

    write_frame(
        stream,
        &PeerFrame::Hello {
            version: PEER_PROTOCOL_VERSION,
            name: intro.name.clone(),
            can_launch: intro.can_launch,
            accelerator: intro.accelerator.clone(),
            os: intro.os.clone(),
            addresses: local.to_vec(),
            version_max: intro.version_max,
            vouched: intro.vouched.clone(),
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

/// This agent's fleet digest: one [`VouchedPeer`] per pin, in `NodeId` byte
/// order, capped at [`MAX_VOUCHED`].
///
/// Sole readers, by construction: the PIN STORE and this agent's own live
/// report cache. Never the vouch table — an entry that arrived in someone
/// else's digest must not leave in ours, because one-hop knowledge is the
/// structural rule (not a TTL) that stops gossip flooding. Every field is a
/// serialization of what those two sources already hold; nothing is invented,
/// and a peer we have not reached this session is stated plainly as
/// unreachable with nothing claimed on its behalf.
///
/// # Errors
/// If the pin store cannot be read. The caller sends `vouched: None` — "did
/// not say" — in that case, because `Some(vec![])` would affirm having no
/// pins, which is not what an unreadable file means.
pub fn fleet_digest(pins: &PinStore, fleet: &crate::fleet::LocalFleet) -> Result<Vec<VouchedPeer>> {
    let pinned = pins.load()?;
    let reports = fleet.report_snapshot();
    Ok(pinned
        .iter()
        // BTreeMap iteration is NodeId byte order, so WHICH entries survive
        // the cap is deterministic rather than whichever 64 a hash map
        // yielded first.
        .take(MAX_VOUCHED)
        .map(|(id, pin)| {
            let held = reports.get(id);
            let report = held.map(|(r, _)| r);
            // Vitals and their age travel together: vitals without a recorded
            // moment would let the receiver render second-hand data as fresh.
            let vitals = report.and_then(|r| r.vitals.clone());
            let vitals_age_s = match (&vitals, held) {
                (Some(_), Some((_, at))) => Some(at.elapsed().as_secs()),
                _ => None,
            };
            VouchedPeer {
                node: *id,
                name: report.map_or_else(
                    || pin.name.clone(),
                    |r| atlasctl_protocol::fleet::DisplayName::new(&r.name),
                ),
                // Absent report means "it has told us nothing this session":
                // no capability, no tags, no addresses — not remembered
                // values presented as current.
                can_launch: report.is_some_and(|r| r.can_launch),
                accelerator: report.map_or_else(String::new, |r| r.accelerator.clone()),
                os: report.map_or_else(String::new, |r| r.os.clone()),
                addresses: report.map_or_else(Vec::new, |r| r.addresses.clone()),
                link: report.map_or(LinkClass::Unverified, |r| r.link),
                // The report cache holds an entry exactly when the last poll
                // succeeded — failures clear it — so presence IS the answer.
                reachable: report.is_some(),
                vitals,
                vitals_age_s,
            }
        })
        .collect())
}
