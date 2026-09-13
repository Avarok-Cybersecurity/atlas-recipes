// SPDX-License-Identifier: AGPL-3.0-only

//! This machine's side of a bench conversation: its key, its pins, and one
//! runtime to dial on. Every request goes through [`Session::request`], so
//! the address fan-out and the error classification happen in one place.

use super::address::Target;
use super::exit::BenchError;
use atlasctl_agent::identity::{Identity, PinStore};
use atlasctl_agent::peer::bench::{self, Attached};
use atlasctl_agent::peer::link::SelfIntro;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench::JobId;
use atlasctl_protocol::msg::{BenchRep, BenchReq};
use std::net::SocketAddr;
use std::sync::Arc;

/// Where a node answered from, once known.
#[derive(Debug, Clone, Copy)]
pub struct Reached {
    pub id: NodeId,
    pub addr: SocketAddr,
}

pub struct Session {
    identity: Arc<Identity>,
    pins: PinStore,
    intro: SelfIntro,
    rt: tokio::runtime::Runtime,
}

impl Session {
    /// Load this machine's identity and pins from the config directory.
    ///
    /// No agent needs to be running: the CLI dials with the same `agent.key`
    /// the agent would, which is what the node has pinned.
    ///
    /// # Errors
    /// `io` when the config directory or key cannot be read, or no runtime
    /// can start.
    pub fn open() -> Result<Self, BenchError> {
        let dir =
            crate::hostinfo::usable_config_dir().map_err(|e| BenchError::io(format!("{e:#}")))?;
        let identity = Identity::load_or_create(&dir)
            .map_err(|e| BenchError::io(format!("loading this machine's key: {e:#}")))?;
        let pins = PinStore::new(&dir);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| BenchError::io(format!("starting a runtime: {e}")))?;
        Ok(Self {
            identity: Arc::new(identity),
            pins,
            // This process submits work; it is not offering to run a model,
            // whatever the box under it could do.
            intro: SelfIntro::new(false, ""),
            rt,
        })
    }

    /// Send one non-attach request to the first address of `target` that
    /// answers, and return who answered along with the reply.
    ///
    /// Addresses are tried in order; a transient dial failure moves to the
    /// next, any other failure is final — a "not paired" on the first address
    /// is not going to be different on the second.
    ///
    /// # Errors
    /// Classified by [`BenchError::from_link`]; a `Refused` reply is an error
    /// here too, because no command wants to handle it as a value.
    pub fn request(
        &self,
        target: &Target,
        addrs: &[SocketAddr],
        req: &BenchReq,
    ) -> Result<(Reached, BenchRep), BenchError> {
        let mut last: Option<BenchError> = None;
        for addr in addrs {
            match self.rt.block_on(bench::send_bench(
                &self.identity,
                self.pins.clone(),
                *addr,
                &self.intro,
                req,
            )) {
                Ok((id, BenchRep::Refused { by, refusal })) => {
                    return Err(BenchError::refused(by, &refusal).at(&target.given).by(id));
                }
                Ok((id, rep)) => return Ok((Reached { id, addr: *addr }, rep)),
                Err(e) => {
                    let classified = BenchError::from_link(&target.given, e);
                    let keep_trying = classified.obj.code == "unreachable";
                    last = Some(classified);
                    if !keep_trying {
                        break;
                    }
                }
            }
        }
        Err(last
            .unwrap_or_else(|| BenchError::unreachable("no addresses to dial").at(&target.given)))
    }

    /// Attach to `job` at a known address.
    ///
    /// # Errors
    /// Classified by [`BenchError::from_link`].
    pub fn attach(
        &self,
        target: &Target,
        addr: SocketAddr,
        job: &JobId,
        from_seq: u64,
    ) -> Result<Attached, BenchError> {
        self.rt
            .block_on(bench::attach(
                &self.identity,
                self.pins.clone(),
                addr,
                &self.intro,
                job,
                from_seq,
            ))
            .map_err(|e| BenchError::from_link(&target.given, e))
    }

    /// Drive one step of an attached stream.
    ///
    /// # Errors
    /// The stream's own error, unclassified — the caller decides whether a
    /// re-attach is still within budget.
    pub fn next_event(
        &self,
        attached: &mut Attached,
    ) -> anyhow::Result<Option<atlasctl_protocol::msg::BenchEvent>> {
        self.rt.block_on(attached.next())
    }
}
