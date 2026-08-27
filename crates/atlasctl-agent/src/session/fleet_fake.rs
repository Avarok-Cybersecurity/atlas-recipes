// SPDX-License-Identifier: AGPL-3.0-only

//! A fleet fake shared by the pairing test modules.
//!
//! Its own file because two test modules need it, and a fake copied into both
//! is a fake that drifts: the copy nobody updated goes on asserting the old
//! contract and passing.
//!
//! It stands in for the network and the pin store together — the real `pair`
//! needs a second machine and a TLS handshake. It does not fake the logic under
//! test; every decision exercised through it is the real `Session`.

use crate::fleet::{FleetView, PairOutcome};
use atlasctl_protocol::fleet::{NodeDescriptor, NodeId};
use std::sync::Mutex;

/// A fleet that records what it was asked to trust.
pub(super) struct RecordingFleet {
    pub(super) outcome: PairOutcome,
    trusted: Mutex<Vec<String>>,
    /// Nodes whose pin was written WITH the controller grant.
    granted: Mutex<Vec<NodeId>>,
    pub(super) fail_pin: bool,
    /// Make the ceremony itself fail, as an unreachable address does.
    ///
    /// Behind a lock so a test can flip it mid-session: the interesting case is
    /// an attempt that fails AFTER one succeeded, which is when a stale
    /// exchange could be left confirmable.
    fail_exchange: Mutex<bool>,
}

impl RecordingFleet {
    pub(super) fn new(node: NodeId) -> Self {
        Self {
            outcome: PairOutcome {
                node,
                public_key: "1a2b3c".repeat(10),
                name: "spark-b".to_owned(),
                address: "10.10.10.2:8765".to_owned(),
                verification: "amber-koala-drift".to_owned(),
            },
            trusted: Mutex::new(Vec::new()),
            granted: Mutex::new(Vec::new()),
            fail_pin: false,
            fail_exchange: Mutex::new(false),
        }
    }

    pub(super) fn keys_pinned(&self) -> Vec<String> {
        self.trusted.lock().expect("poisoned").clone()
    }

    pub(super) fn grants(&self) -> Vec<NodeId> {
        self.granted.lock().expect("poisoned").clone()
    }
}

impl FleetView for RecordingFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        Vec::new()
    }

    fn pair(&self, _node: NodeId, _code: &str) -> anyhow::Result<PairOutcome> {
        self.exchange()
    }

    fn pair_at(&self, _target: &str, _code: &str) -> anyhow::Result<PairOutcome> {
        self.exchange()
    }

    fn trust(&self, outcome: &PairOutcome, allow_control: bool) -> anyhow::Result<()> {
        if self.fail_pin {
            anyhow::bail!("disk full");
        }
        self.trusted
            .lock()
            .expect("poisoned")
            .push(outcome.public_key.clone());
        if allow_control {
            self.granted.lock().expect("poisoned").push(outcome.node);
        }
        Ok(())
    }

    fn unpair(&self, _node: NodeId) -> anyhow::Result<bool> {
        Ok(false)
    }
}

impl RecordingFleet {
    /// Every subsequent ceremony fails, as an unreachable address would.
    pub(super) fn start_failing(&self) {
        *self.fail_exchange.lock().expect("poisoned") = true;
    }

    fn exchange(&self) -> anyhow::Result<PairOutcome> {
        if *self.fail_exchange.lock().expect("poisoned") {
            anyhow::bail!("nothing answered at that address");
        }
        Ok(self.outcome.clone())
    }
}
