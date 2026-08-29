// SPDX-License-Identifier: AGPL-3.0-only

//! Running the pairing ceremony on the agent's own runtime.
//!
//! The counterpart to [`crate::peertransport`]: the fleet decides who to pair
//! with, this owns the socket. Everything about *what a pairing means* lives in
//! [`atlasctl_agent::peer::join`], which the CLI calls directly — so the
//! browser-driven path and `atlasctl peer add` are one exchange, not two
//! implementations that could diverge into a strong one and a weak one.

use anyhow::Result;
use atlasctl_agent::fleet::PeerPairing;
use atlasctl_agent::identity::{Identity, PinStore};
use atlasctl_agent::peer::pair::Paired;
use std::net::SocketAddr;
use std::sync::Arc;

/// Pairs with peers on the agent's runtime.
pub struct RuntimePeerPairing {
    identity: Arc<Identity>,
    pins: PinStore,
}

impl std::fmt::Debug for RuntimePeerPairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimePeerPairing").finish_non_exhaustive()
    }
}

impl RuntimePeerPairing {
    /// Build one.
    #[must_use]
    pub fn new(identity: Arc<Identity>, pins: PinStore) -> Self {
        Self { identity, pins }
    }
}

impl PeerPairing for RuntimePeerPairing {
    fn pair<'a>(
        &'a self,
        addr: SocketAddr,
        code: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Paired>> + Send + 'a>> {
        // Awaited, not blocked: this runs inside a task on the agent's runtime,
        // and a ceremony is a network round trip. Holding a worker for it would
        // stall every other session and the timers meant to bound them.
        Box::pin(atlasctl_agent::peer::join::dial_and_pair(
            &self.identity,
            self.pins.clone(),
            addr,
            code,
        ))
    }
}
