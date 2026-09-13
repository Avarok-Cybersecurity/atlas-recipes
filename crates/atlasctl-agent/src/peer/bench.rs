// SPDX-License-Identifier: AGPL-3.0-only

//! The client half of the benchmark-job frames: asking a node to run a gate,
//! and following the job it runs.
//!
//! Every helper dials through the pinned-mTLS choke point. Two things differ
//! from `control.rs`: the caller knows an ADDRESS, not always a fingerprint —
//! `--with-nodes 10.10.10.2` — so [`dial_any`] accepts whichever pinned peer
//! answers and returns who it was; and an attached job is a long-lived
//! stream, not a request with a 60-second budget, so [`attach`] hands back a
//! reader with an idle timeout rather than a single reply.

use super::link::{DIAL_TIMEOUT, Hello, SelfIntro, exchange_hello};
use super::tls::{PinnedPeerVerifier, client_config, peer_identity};
use super::wire::{PEER_PROTOCOL_MAX, PEER_PROTOCOL_VERSION, PeerFrame, read_frame, write_frame};
use crate::identity::{Identity, PinStore};
use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::JobId;
use atlasctl_protocol::msg::{BenchEvent, BenchRep, BenchReq, EventKind};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// The lowest peer-protocol version that carries bench frames.
pub const BENCH_MIN_VERSION: u32 = 3;
/// How long a node has to answer a non-attach bench request. Submission,
/// status, cancel and an artifact chunk are all quick; the work itself is
/// followed through `attach`.
pub const BENCH_ANSWER_BUDGET: Duration = Duration::from_secs(30);
/// How long an attached stream may be silent before the reader gives up.
/// The node heartbeats every [`HEARTBEAT`], so silence this long is a dead
/// link, not a quiet job.
pub const ATTACH_IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// How often a node writes a heartbeat on an attached stream.
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// Unsolicited frames tolerated while waiting for a reply.
const SKIP_BUDGET: usize = 8;

pub type Tls = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;

/// Refuse, by name, a peer that cannot decode bench frames.
///
/// # Errors
/// If the hello advertises less than [`BENCH_MIN_VERSION`].
pub fn ensure_bench_capable(hello: &Hello, peer: NodeId) -> Result<()> {
    let version_max = hello.version_max.unwrap_or(PEER_PROTOCOL_VERSION);
    if version_max < BENCH_MIN_VERSION {
        bail!(
            "{} ({}) speaks peer protocol {version_max} and cannot carry bench frames \
             (this build speaks up to {PEER_PROTOCOL_MAX}); upgrade it",
            hello.name,
            peer.short()
        );
    }
    Ok(())
}

/// Dial `addr` and accept whichever PINNED peer answers, returning who it was.
///
/// The verifier still refuses any certificate that is not in the pin store,
/// so this is not weaker than `link::dial` — it only drops the requirement
/// that the caller already know which pin lives at the address.
///
/// # Errors
/// If the address does not answer, the handshake fails, or the peer is not
/// pinned.
pub async fn dial_any(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
) -> Result<(Tls, NodeId)> {
    let cfg = client_config(identity, PinnedPeerVerifier::pinned(pins, None))?;
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
    let peer_id = {
        let (_, conn) = tls.get_ref();
        let cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{addr} sent no certificate"))?;
        peer_identity(&cert)?.0
    };
    Ok((tls, peer_id))
}

/// Dial `addr`, introduce ourselves, and send one non-attach bench request.
///
/// # Errors
/// If the peer cannot be reached, is not pinned, speaks a protocol below
/// bench, or does not answer within [`BENCH_ANSWER_BUDGET`].
pub async fn send_bench(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    intro: &SelfIntro,
    req: &BenchReq,
) -> Result<(NodeId, BenchRep)> {
    debug_assert!(
        !matches!(req, BenchReq::Attach { .. }),
        "attach has its own path"
    );
    let (mut tls, peer) = dial_any(identity, pins, addr).await?;
    let hello = exchange_hello(&mut tls, addr, intro, &[]).await?;
    ensure_bench_capable(&hello, peer)?;
    write_frame(&mut tls, &PeerFrame::Bench { req: req.clone() }).await?;
    let rep = bench_reply(&mut tls, addr, BENCH_ANSWER_BUDGET).await?;
    Ok((peer, rep))
}

/// Read exactly one `BenchReply`, skipping interleaved vitals.
async fn bench_reply<S>(tls: &mut S, addr: SocketAddr, budget: Duration) -> Result<BenchRep>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let deadline = tokio::time::Instant::now() + budget;
    for _ in 0..SKIP_BUDGET {
        let frame = match tokio::time::timeout_at(deadline, read_frame(tls)).await {
            Ok(f) => f?,
            Err(_) => bail!("{addr} did not answer the bench request within {budget:?}"),
        };
        match frame {
            PeerFrame::Vitals { .. } => continue,
            PeerFrame::BenchReply { rep } => return Ok(rep),
            other => bail!("expected a bench reply from {addr}, got {other:?}"),
        }
    }
    bail!("{addr} kept sending vitals instead of answering the bench request")
}

/// An attached job stream.
pub struct Attached {
    tls: Tls,
    addr: SocketAddr,
    /// The peer that answered.
    pub peer: NodeId,
    /// Highest journaled seq seen (heartbeats do not advance it).
    pub last_seq: u64,
    done: bool,
}

impl Attached {
    /// The next journaled event, or `None` after `Done`.
    ///
    /// Heartbeats are consumed here and never returned: they only prove the
    /// link is alive. Silence longer than [`ATTACH_IDLE_TIMEOUT`] is an
    /// error the caller re-attaches from `last_seq + 1` to recover from.
    ///
    /// # Errors
    /// On a dead link, a malformed frame, or a seq that went backwards.
    pub async fn next(&mut self) -> Result<Option<BenchEvent>> {
        if self.done {
            return Ok(None);
        }
        loop {
            let frame = match tokio::time::timeout(ATTACH_IDLE_TIMEOUT, read_frame(&mut self.tls))
                .await
            {
                Ok(f) => f?,
                Err(_) => bail!(
                    "{} went silent for {ATTACH_IDLE_TIMEOUT:?} on an attached job (last seq {})",
                    self.addr,
                    self.last_seq
                ),
            };
            match frame {
                PeerFrame::Vitals { .. } => continue,
                PeerFrame::BenchEvent { event } => {
                    if matches!(event.kind, EventKind::Heartbeat { .. }) {
                        continue;
                    }
                    if event.seq <= self.last_seq {
                        // A replay overlap; the node re-sent what we have.
                        continue;
                    }
                    if self.last_seq != 0 && event.seq != self.last_seq + 1 {
                        bail!(
                            "{} skipped from seq {} to {} — re-attach from {}",
                            self.addr,
                            self.last_seq,
                            event.seq,
                            self.last_seq + 1
                        );
                    }
                    self.last_seq = event.seq;
                    if matches!(event.kind, EventKind::Done { .. }) {
                        self.done = true;
                    }
                    return Ok(Some(event));
                }
                PeerFrame::BenchReply {
                    rep: BenchRep::Refused { refusal, .. },
                } => {
                    bail!("{} refused the attach: {refusal}", self.addr)
                }
                other => bail!("expected a bench event from {}, got {other:?}", self.addr),
            }
        }
    }
}

/// Dial `addr` and attach to `job` from `from_seq`.
///
/// # Errors
/// If the peer cannot be reached, is not pinned, speaks a protocol below
/// bench, or refuses the attach.
pub async fn attach(
    identity: &Identity,
    pins: PinStore,
    addr: SocketAddr,
    intro: &SelfIntro,
    job: &JobId,
    from_seq: u64,
) -> Result<Attached> {
    let (mut tls, peer) = dial_any(identity, pins, addr).await?;
    let hello = exchange_hello(&mut tls, addr, intro, &[]).await?;
    ensure_bench_capable(&hello, peer)?;
    write_frame(
        &mut tls,
        &PeerFrame::Bench {
            req: BenchReq::Attach {
                job: job.clone(),
                from_seq,
            },
        },
    )
    .await?;
    Ok(Attached {
        tls,
        addr,
        peer,
        last_seq: from_seq.saturating_sub(1),
        done: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(version_max: Option<u32>) -> Hello {
        Hello {
            name: "spark-43fa".into(),
            can_launch: true,
            accelerator: String::new(),
            os: String::new(),
            addresses: vec![],
            version_max,
            vouched: None,
        }
    }

    #[test]
    fn a_v3_peer_is_bench_capable_and_older_ones_are_refused_by_name() {
        let peer = NodeId::parse(&"ab".repeat(32)).unwrap();
        assert!(ensure_bench_capable(&hello(Some(3)), peer).is_ok());
        assert!(ensure_bench_capable(&hello(Some(PEER_PROTOCOL_MAX)), peer).is_ok());
        for older in [Some(2), Some(1), None] {
            let err = ensure_bench_capable(&hello(older), peer)
                .unwrap_err()
                .to_string();
            assert!(err.contains("spark-43fa"), "{err}");
            assert!(err.contains("cannot carry bench frames"), "{err}");
        }
    }
}
