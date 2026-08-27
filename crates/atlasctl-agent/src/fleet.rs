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

#[cfg(test)]
#[path = "fleet/pairing_tests.rs"]
mod pairing_tests;

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

    /// Run the ceremony against an address the operator typed.
    ///
    /// Distinct from [`Self::pair`] because there is no expected identity to
    /// verify against — nothing was discovered, so the caller learns who
    /// answered from the returned outcome. That is not weaker: `pair`'s
    /// identity check exists to stop a machine at a beacon's address being
    /// pinned under the identity of the one the operator *selected*, and here
    /// the operator selected an address, not an identity.
    ///
    /// # Errors
    /// If the target does not resolve, is unreachable, or the ceremony fails.
    fn pair_at(&self, target: &str, code: &str) -> Result<PairOutcome>;

    /// Write the pin for an exchange a human has accepted.
    ///
    /// Split from [`Self::pair`] so nothing is trusted until somebody says the
    /// words matched. The caller holds the [`PairOutcome`] in the meantime; if
    /// it never confirms, no pin is ever written and there is nothing to undo.
    ///
    /// `allow_control` writes the `controller` grant with the pin — one
    /// atomic decision, taken at the moment a human is already deciding to
    /// trust the machine. It defaults to nothing: pairing authenticates, and
    /// only this explicit flag (or `atlasctl peer grant-control`) authorizes.
    ///
    /// # Errors
    /// If the pin store cannot be written.
    fn trust(&self, outcome: &PairOutcome, allow_control: bool) -> Result<()>;

    /// Drop trust in a peer. Returns whether it was trusted.
    ///
    /// # Errors
    /// If the pin store cannot be written.
    fn unpair(&self, node: NodeId) -> Result<bool>;
}

/// Dialling a peer and completing the pairing ceremony against it.
///
/// A port, for the same reason [`crate::transport::RankTransport`] is one: the
/// fleet decides *who* to pair with and *what it means*, and something else
/// owns the socket and the runtime. It also makes the refusals below testable
/// without a second machine.
pub trait PeerPairing: Send + Sync {
    /// Pair with the machine at `addr` using `code`, as the initiator.
    ///
    /// Must not record trust; the caller decides that.
    ///
    /// # Errors
    /// If the machine cannot be reached or the ceremony fails.
    fn pair(&self, addr: std::net::SocketAddr, code: &str) -> Result<crate::peer::pair::Paired>;
}

/// A completed exchange, not yet trusted.
///
/// Everything [`FleetView::trust`] needs to write a pin, held so the write can
/// wait for a human. Before two-phase pairing this was written the moment the
/// ceremony returned and the words were shown afterwards, which made comparing
/// them a formality — the machine an operator went on to reject had already
/// been trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairOutcome {
    /// Who completed the exchange.
    pub node: NodeId,
    /// The key that would be pinned, hex encoded, exactly as the ceremony
    /// produced it.
    pub public_key: String,
    /// What that machine calls itself.
    pub name: String,
    /// Where it was reached, recorded with the pin so a later dial has a
    /// starting point.
    pub address: String,
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
    running_source: Option<Box<dyn vitals::RunningSource>>,
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
    ///
    /// The instant is when the report was recorded, so a vouch built from it
    /// can state how old its vitals are instead of presenting them as fresh.
    reports: Mutex<BTreeMap<NodeId, (crate::peer::link::PeerReport, Instant)>>,
    /// Second-hand knowledge: what pinned peers say about THEIR pins.
    /// Everything about it lives in [`vouched`], including the rule that it
    /// never feeds this agent's own digest.
    vouches: vouched::VouchTable,
    /// How to run the ceremony, when this agent can.
    pairing: Option<Box<dyn PeerPairing>>,
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
            running_source: None,
            seen: Mutex::new(BTreeMap::new()),
            alerts: Mutex::new(BTreeMap::new()),
            running: Mutex::new(None),
            reports: Mutex::new(BTreeMap::new()),
            vouches: vouched::VouchTable::default(),
            pairing: None,
        }
    }

    /// Give this fleet a way to actually run the pairing ceremony.
    ///
    /// Without it `pair` refuses rather than pretending: a pairing that
    /// reported success without a key exchange would write a pin that means
    /// nothing, which is worse than not pairing at all.
    #[must_use]
    pub fn with_pairing(mut self, p: Box<dyn PeerPairing>) -> Self {
        self.pairing = Some(p);
        self
    }

    /// Attach a source for what this machine is serving.
    #[must_use]
    pub fn with_running(mut self, source: Box<dyn vitals::RunningSource>) -> Self {
        self.running_source = Some(source);
        self
    }

    /// This machine's own addresses, with their subnets.
    ///
    /// Told to peers over the authenticated channel, because a rendezvous
    /// address has to be one every rank can reach and only a node knows which
    /// links it is attached to.
    #[must_use]
    pub fn local_addresses(&self) -> Vec<NodeAddress> {
        self.local_addresses.clone()
    }

    /// Re-read what is running here, and report whether it changed.
    ///
    /// Returns the new value only when it differs, so a caller can push an
    /// event on a change instead of on every tick.
    pub fn refresh_running(&self) -> Option<Option<String>> {
        let found = self.running_source.as_ref()?.running();
        let mut held = self.running.lock().ok()?;
        if *held == found {
            return None;
        }
        *held = found.clone();
        Some(found)
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
            r.insert(report.node, (report, Instant::now()));
        }
    }

    /// Forget what a peer said, because it stopped answering.
    ///
    /// Its vouches deliberately stay. Routing is already safe without dropping
    /// them — `choose_voucher` requires a live report from the voucher — so
    /// dropping them buys nothing there, and costs the operator their fleet: a
    /// machine reached through this peer would VANISH the moment it missed a
    /// poll, which reads as "you never paired that". It stays, `Vouched` with
    /// `reached_via: None`, so the page can say it is behind a machine that is
    /// not answering. Second-hand vitals age out on their own stated age.
    ///
    /// `unpair` is the other case and does clear them: a peer no longer
    /// trusted is not trusted to have said anything.
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
        self.prune_vouches();
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
            os: crate::discovery::local_os(),
            vitals: Some(vitals),
            alerts: self
                .alerts
                .lock()
                .ok()
                .and_then(|a| a.get(&self.identity.id()).cloned())
                .unwrap_or_default(),
            running: self.running.lock().ok().and_then(|r| r.clone()),
            // This machine: identity is first-hand by definition, and a verb
            // aimed at it never rides a relay.
            vouched_by: None,
            reached_via: None,
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
    controller: bool,
) -> Result<()> {
    pins.add(Pin {
        id: node,
        public_key: public_key_hex.to_owned(),
        name,
        paired_at: now_unix,
        last_address,
        // Pairing authenticates; it does not authorize control. `controller`
        // is true only when a human said so in the same breath as trusting
        // the machine (`allow_control` on the confirm or the join window) —
        // never as a side effect of the ceremony itself, and otherwise the
        // grant stays a separate act (`atlasctl peer grant-control`).
        controller,
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
pub mod routing;
pub mod vitals;
pub mod vouched;

#[cfg(test)]
#[path = "fleet/routing_tests.rs"]
mod routing_tests;

#[cfg(test)]
#[path = "fleet/vouched_tests.rs"]
mod vouched_tests;

pub use vitals::{
    DockerRunning, RunningSource, SystemVitals, VitalsSource, docker_probe_argv,
    running_probe_argv, vitals_from_device,
};
