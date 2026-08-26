// SPDX-License-Identifier: AGPL-3.0-only

//! What this agent knows about the machines around it.
//!
//! [`FleetView`] is what a session asks; [`LocalFleet`] is the implementation
//! that owns the discovery table, the pin store and this machine's own
//! description. Splitting them is what lets the whole browser-facing fleet
//! surface — including every refusal — be tested without a network, a second
//! machine, or a running mDNS responder.
//!
//! One rule runs through all of it: **a beacon is not a permission.** Anything
//! learned from discovery lands here as [`PairingState::Discovered`], carries
//! no vitals, and can be launched on by nobody. Only the pairing ceremony moves
//! a node out of that state, and only the pin store records it.

use crate::discovery::Beacon;
use crate::identity::{Identity, Pin, PinStore};
use anyhow::Result;
use atlasctl_protocol::fleet::{
    DisplayName, Launchability, NodeAddress, NodeAlert, NodeDescriptor, NodeId, NodeVitals,
    PairingState,
};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

/// A node absent for longer than this is reported as unreachable.
///
/// Generous on purpose. mDNS records re-resolve on their own schedule rather
/// than on ours, and 30 seconds turned out to be shorter than the refresh
/// interval on a quiet network — a paired Spark that was up and answering
/// showed as unreachable simply because nothing had re-announced it yet. One
/// missed refresh must not make a machine blink out of someone's interface.
pub const UNREACHABLE_AFTER: Duration = Duration::from_secs(120);

/// What a session may ask about the fleet.
pub trait FleetView: Send + Sync {
    /// Every node this agent knows about, local first.
    fn nodes(&self) -> Vec<NodeDescriptor>;

    /// Run the pairing ceremony against a discovered peer.
    ///
    /// # Errors
    /// If the peer is unknown, unreachable, or the ceremony fails.
    fn pair(&self, node: NodeId, code: &str) -> Result<PairOutcome>;

    /// Drop trust in a peer. Returns whether it was trusted.
    ///
    /// # Errors
    /// If the pin store cannot be written.
    fn unpair(&self, node: NodeId) -> Result<bool>;
}

/// The result of a successful pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairOutcome {
    /// Who was paired.
    pub node: NodeId,
    /// Short words both humans compare before trusting.
    pub verification: String,
}

/// One entry in the discovery table.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) beacon: Beacon,
    pub(crate) at: Instant,
}

/// This agent's view of the fleet.
pub struct LocalFleet {
    identity: Identity,
    pins: PinStore,
    /// This machine's own addresses, from the fabric provider.
    local_addresses: Vec<NodeAddress>,
    /// Whether this machine can run a model, and why not if it cannot.
    launchability: Launchability,
    /// Coarse accelerator tag for display.
    accelerator: String,
    /// Display name for this machine.
    name: DisplayName,
    /// Where this machine's vitals come from, when anything can supply them.
    vitals: Option<Box<dyn VitalsSource>>,
    /// Beacons seen, by node id.
    seen: Mutex<BTreeMap<NodeId, Seen>>,
    /// Alerts currently raised, by node.
    alerts: Mutex<BTreeMap<NodeId, Vec<NodeAlert>>>,
    /// What is running locally, if anything.
    running: Mutex<Option<String>>,
    /// What paired peers have told us over the authenticated channel.
    ///
    /// Separate from `seen`, and it must stay separate: a beacon says where a
    /// machine claims to be, while this is what a machine holding the key we
    /// pinned actually said. Only the second is evidence.
    reports: Mutex<BTreeMap<NodeId, crate::peer::link::PeerReport>>,
}

impl std::fmt::Debug for LocalFleet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalFleet")
            .field("id", &self.identity.id())
            .finish_non_exhaustive()
    }
}

impl LocalFleet {
    /// Build a view of the fleet from this machine's facts.
    #[must_use]
    pub fn new(
        identity: Identity,
        pins: PinStore,
        name: DisplayName,
        local_addresses: Vec<NodeAddress>,
        launchability: Launchability,
        accelerator: String,
    ) -> Self {
        Self {
            identity,
            pins,
            local_addresses,
            launchability,
            accelerator,
            name,
            vitals: None,
            seen: Mutex::new(BTreeMap::new()),
            alerts: Mutex::new(BTreeMap::new()),
            running: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
        }
    }

    /// Attach a source for this machine's vitals.
    #[must_use]
    pub fn with_vitals(mut self, source: Box<dyn VitalsSource>) -> Self {
        self.vitals = Some(source);
        self
    }

    /// Whether this machine can run a model.
    #[must_use]
    pub fn can_launch(&self) -> bool {
        self.launchability.can_launch
    }

    /// This agent's identity.
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.identity.id()
    }

    /// Record a beacon. Never grants trust.
    pub fn observe(&self, beacon: Beacon) {
        // A node cannot advertise itself into your pin store, and it cannot
        // advertise away a pin you already hold: this only ever writes into the
        // sightings table.
        if beacon.id == self.identity.id() {
            return;
        }
        // A sighting of a peer we trust also refreshes where we think it is,
        // so the address outlives this process.
        if let Some(addr) = beacon.addresses.first().map(ToString::to_string) {
            let _ = remember_address(&self.pins, beacon.id, &addr);
        }
        if let Ok(mut seen) = self.seen.lock() {
            seen.insert(
                beacon.id,
                Seen {
                    beacon,
                    at: Instant::now(),
                },
            );
        }
    }

    /// Record what a peer said over the authenticated channel.
    pub fn record_report(&self, report: crate::peer::link::PeerReport) {
        if let Ok(mut r) = self.reports.lock() {
            r.insert(report.node, report);
        }
    }

    /// Forget what a peer said, because it stopped answering.
    pub fn clear_report(&self, node: NodeId) {
        if let Ok(mut r) = self.reports.lock() {
            r.remove(&node);
        }
    }

    /// Peers this agent should try to reach, with the address to use.
    ///
    /// # Errors
    /// If the pin store cannot be read.
    pub fn dialable_peers(&self) -> Result<Vec<(NodeId, String)>> {
        let pinned = self.pins.load()?;
        let seen = self.lock_seen();
        Ok(pinned
            .iter()
            .filter_map(|(id, pin)| {
                let addr = seen
                    .as_ref()
                    .and_then(|s| s.get(id))
                    .and_then(|s| s.beacon.addresses.first().map(ToString::to_string))
                    .or_else(|| pin.last_address.clone())?;
                Some((*id, addr))
            })
            .collect())
    }

    /// How this machine classifies the link an address sits on.
    ///
    /// Asked of our own fabric probe, never of the peer: a peer's opinion of
    /// its own link says nothing about the path between us.
    #[must_use]
    pub fn classify_peer_address(&self, addr: &str) -> atlasctl_protocol::fleet::LinkClass {
        use atlasctl_protocol::fleet::LinkClass;
        // Same /24 as one of our own interfaces is a reasonable proxy for
        // "reached over that interface" without shelling out to a routing
        // table on every poll.
        let prefix = |a: &str| a.rsplit_once('.').map(|(head, _)| head.to_owned());
        let want = prefix(addr);
        self.local_addresses
            .iter()
            .find(|a| prefix(&a.addr) == want)
            .map_or(LinkClass::Unverified, |a| a.class)
    }

    /// Note what is running locally, so the fleet view can report it.
    pub fn set_running(&self, recipe: Option<String>) {
        if let Ok(mut r) = self.running.lock() {
            *r = recipe;
        }
    }

    /// Raise or replace the alerts for a node.
    pub fn set_alerts(&self, node: NodeId, alerts: Vec<NodeAlert>) {
        if let Ok(mut a) = self.alerts.lock() {
            if alerts.is_empty() {
                a.remove(&node);
            } else {
                a.insert(node, alerts);
            }
        }
    }

    /// Forget sightings older than [`UNREACHABLE_AFTER`] for peers that are not
    /// pinned. A pinned peer stays in the list, marked unreachable, because it
    /// is still part of your fleet even when it is switched off.
    pub fn prune(&self) {
        let pinned = self.pins.load().unwrap_or_default();
        if let Ok(mut seen) = self.seen.lock() {
            seen.retain(|id, s| pinned.contains_key(id) || s.at.elapsed() < UNREACHABLE_AFTER);
        }
    }

    pub(crate) fn lock_seen(&self) -> Option<MutexGuard<'_, BTreeMap<NodeId, Seen>>> {
        self.seen.lock().ok()
    }

    /// This machine's id and a fresh vitals sample, when anything can supply
    /// one.
    ///
    /// Separate from [`Self::local_node`] because the vitals pusher wants only
    /// the sample: rebuilding the whole descriptor once a second would send the
    /// unchanged parts over and over.
    #[must_use]
    pub fn local_vitals_and_id(&self) -> Option<(NodeId, NodeVitals)> {
        let source = self.vitals.as_ref()?;
        Some((self.identity.id(), source.vitals()))
    }

    /// This machine, as a node.
    fn local_node(&self) -> NodeDescriptor {
        let vitals = self
            .vitals
            .as_ref()
            .map_or_else(NodeVitals::default, |v| v.vitals());
        NodeDescriptor {
            id: self.identity.id(),
            name: self.name.clone(),
            is_local: true,
            pairing: PairingState::Paired,
            addresses: self.local_addresses.clone(),
            launchability: self.launchability.clone(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            accelerator: self.accelerator.clone(),
            vitals: Some(vitals),
            alerts: self
                .alerts
                .lock()
                .ok()
                .and_then(|a| a.get(&self.identity.id()).cloned())
                .unwrap_or_default(),
            running: self.running.lock().ok().and_then(|r| r.clone()),
        }
    }
}

/// Record a completed pairing.
///
/// Separate from [`FleetView::pair`] so the ceremony's transport and its
/// bookkeeping are not tangled: whatever drives the exchange calls this once,
/// and only once key confirmation has passed.
///
/// # Errors
/// If the pin store cannot be written.
pub fn record_pairing(
    pins: &PinStore,
    node: NodeId,
    public_key_hex: &str,
    name: DisplayName,
    now_unix: u64,
    last_address: Option<String>,
) -> Result<()> {
    pins.add(Pin {
        id: node,
        public_key: public_key_hex.to_owned(),
        name,
        paired_at: now_unix,
        last_address,
    })
}

/// Remember where a paired peer was last seen.
///
/// Called when a beacon refreshes, so the address survives an agent restart.
///
/// # Errors
/// If the pin store cannot be read or written.
pub fn remember_address(pins: &PinStore, node: NodeId, addr: &str) -> Result<()> {
    let mut all = pins.load()?;
    if let Some(pin) = all.get_mut(&node)
        && pin.last_address.as_deref() != Some(addr)
    {
        pin.last_address = Some(addr.to_owned());
        let updated = pin.clone();
        pins.add(updated)?;
    }
    Ok(())
}

/// Vitals a provider could not supply at all.
#[must_use]
pub fn no_vitals() -> NodeVitals {
    NodeVitals::default()
}

mod listing;
pub mod vitals;

pub use vitals::{SystemVitals, VitalsSource, docker_probe_argv, vitals_from_device};
