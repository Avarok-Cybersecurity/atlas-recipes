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
/// Two discovery intervals, deliberately: one missed refresh on a busy wireless
/// network is normal, and a node that blinks out of someone's interface every
/// few seconds is worse than one that lingers a moment too long.
pub const UNREACHABLE_AFTER: Duration = Duration::from_secs(30);

/// Supplies this machine's vitals.
///
/// A trait rather than a call into the telemetry functions directly, so the
/// fleet view can be tested against a machine that answers everything, a
/// machine that answers nothing, and the GB10 case where the memory questions
/// have no answer at all.
pub trait VitalsSource: Send + Sync {
    /// The current sample.
    fn vitals(&self) -> NodeVitals;
}

/// Turns a device sample into node vitals.
///
/// The `Option -> Metric` conversion is where "absent" is preserved: a field
/// the hardware cannot answer becomes `Metric::Unsupported`, never zero.
#[must_use]
pub fn vitals_from_device(
    d: &atlasctl_protocol::telemetry::DeviceStats,
    disk_free_bytes: Option<f64>,
    docker_ok: bool,
    uptime_s: u64,
    healthy_clock_mhz: Option<u32>,
) -> NodeVitals {
    use atlasctl_protocol::fleet::Metric;
    let used_frac = match (d.memory_used_bytes, d.memory_total_bytes) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a memory fraction is displayed to two significant figures"
        )]
        (Some(u), Some(t)) if t > 0 => Metric::reading(u as f64 / t as f64),
        _ => Metric::Unsupported,
    };
    NodeVitals {
        accelerator_util: d.gpu_util_pct.into(),
        sm_clock_mhz: d.sm_clock_mhz.map(f64::from).into(),
        sm_clock_healthy_mhz: healthy_clock_mhz,
        temperature_c: d.temperature_c.into(),
        power_w: d.power_w.into(),
        memory_used_frac: used_frac,
        #[expect(
            clippy::cast_precision_loss,
            reason = "byte counts are displayed in gigabytes"
        )]
        memory_total_bytes: d.memory_total_bytes.map(|b| b as f64).into(),
        disk_free_bytes: disk_free_bytes.into(),
        docker_ok,
        agent_uptime_s: uptime_s,
    }
}

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
struct Seen {
    beacon: Beacon,
    at: Instant,
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
        }
    }

    /// Attach a source for this machine's vitals.
    #[must_use]
    pub fn with_vitals(mut self, source: Box<dyn VitalsSource>) -> Self {
        self.vitals = Some(source);
        self
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

    fn lock_seen(&self) -> Option<MutexGuard<'_, BTreeMap<NodeId, Seen>>> {
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

impl FleetView for LocalFleet {
    fn nodes(&self) -> Vec<NodeDescriptor> {
        let mut out = vec![self.local_node()];
        let pinned = self.pins.load().unwrap_or_default();
        let seen = self.lock_seen();
        let alerts = self.alerts.lock().ok();

        // Start from the pin store, so a paired machine that is switched off is
        // still listed. A fleet that forgets its members when they sleep is not
        // a fleet.
        for (id, pin) in &pinned {
            let sighting = seen.as_ref().and_then(|s| s.get(id));
            let fresh = sighting.is_some_and(|s| s.at.elapsed() < UNREACHABLE_AFTER);
            out.push(NodeDescriptor {
                id: *id,
                name: sighting.map_or_else(|| pin.name.clone(), |s| s.beacon.name.clone()),
                is_local: false,
                pairing: if fresh {
                    PairingState::Paired
                } else {
                    PairingState::Unreachable
                },
                addresses: sighting
                    .map(|s| addresses_of(&s.beacon))
                    .unwrap_or_default(),
                launchability: sighting.map_or_else(
                    || Launchability::no("not reachable right now"),
                    |s| {
                        if s.beacon.can_launch {
                            Launchability::yes()
                        } else {
                            Launchability::no("this node reports it cannot run models")
                        }
                    },
                ),
                agent_version: String::new(),
                accelerator: sighting.map_or_else(String::new, |s| s.beacon.accelerator.clone()),
                // Vitals arrive over the authenticated peer channel, never from
                // a beacon, so a peer we have not spoken to reports none.
                vitals: None,
                alerts: alerts
                    .as_ref()
                    .and_then(|a| a.get(id).cloned())
                    .unwrap_or_default(),
                running: None,
            });
        }

        // Then anything seen but not pinned: visible, and trusted by nobody.
        if let Some(seen) = seen.as_ref() {
            for (id, s) in seen.iter() {
                if pinned.contains_key(id) {
                    continue;
                }
                out.push(NodeDescriptor {
                    id: *id,
                    name: s.beacon.name.clone(),
                    is_local: false,
                    pairing: PairingState::Discovered,
                    addresses: addresses_of(&s.beacon),
                    launchability: if s.beacon.can_launch {
                        Launchability::yes()
                    } else {
                        Launchability::no("this node reports it cannot run models")
                    },
                    agent_version: String::new(),
                    accelerator: s.beacon.accelerator.clone(),
                    vitals: None,
                    alerts: Vec::new(),
                    running: None,
                });
            }
        }
        out
    }

    fn pair(&self, node: NodeId, code: &str) -> Result<PairOutcome> {
        anyhow::ensure!(
            crate::pairing::looks_like_code(code),
            "a pairing code is {} digits",
            crate::pairing::CODE_DIGITS
        );
        let seen = self
            .lock_seen()
            .and_then(|s| s.get(&node).cloned())
            .ok_or_else(|| anyhow::anyhow!("that node is not visible on this network"))?;

        // The ceremony itself runs over the peer channel; this is the point
        // where that would be driven. Until the peer transport is wired in,
        // refuse rather than pretend: a pairing that silently succeeded without
        // a key exchange would write a pin that means nothing.
        anyhow::bail!(
            "cannot reach {} to pair: the peer channel is not connected",
            seen.beacon.name
        )
    }

    fn unpair(&self, node: NodeId) -> Result<bool> {
        self.pins.remove(node)
    }
}

/// Addresses a beacon advertised, as protocol addresses.
///
/// A beacon carries no link classification — it is unauthenticated, and letting
/// a stranger tell you their link is RoCE would let them talk their way to the
/// top of your preference order. Class is filled in from the local fabric
/// probe once the peer is paired and speaks for itself.
fn addresses_of(beacon: &Beacon) -> Vec<NodeAddress> {
    beacon
        .addresses
        .iter()
        .map(|a| NodeAddress {
            iface: String::new(),
            addr: a.to_string(),
            class: atlasctl_protocol::fleet::LinkClass::Ethernet,
            speed_mbps: None,
            rdma: false,
        })
        .collect()
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
) -> Result<()> {
    pins.add(Pin {
        id: node,
        public_key: public_key_hex.to_owned(),
        name,
        paired_at: now_unix,
    })
}

/// Vitals a provider could not supply at all.
#[must_use]
pub fn no_vitals() -> NodeVitals {
    NodeVitals::default()
}

/// Vitals read from this machine.
///
/// Capabilities are probed once at construction, because the answer does not
/// change while the agent runs and re-probing every second would spawn a
/// process every second. Individual readings are taken per sample.
pub struct SystemVitals {
    runner: std::sync::Arc<dyn atlasctl_core::io::ProcessRunner>,
    caps: atlasctl_protocol::telemetry::TelemetryCaps,
    started: Instant,
    /// Filesystem whose free space matters — images and the model cache.
    disk_path: std::path::PathBuf,
}

impl std::fmt::Debug for SystemVitals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemVitals")
            .field("caps", &self.caps)
            .finish_non_exhaustive()
    }
}

impl SystemVitals {
    /// Probe this machine's telemetry capabilities and start sampling.
    #[must_use]
    pub fn new(
        runner: std::sync::Arc<dyn atlasctl_core::io::ProcessRunner>,
        disk_path: std::path::PathBuf,
    ) -> Self {
        let caps = crate::telemetry::probe(runner.as_ref());
        Self {
            runner,
            caps,
            started: Instant::now(),
            disk_path,
        }
    }

    /// What this machine can answer.
    #[must_use]
    pub const fn caps(&self) -> &atlasctl_protocol::telemetry::TelemetryCaps {
        &self.caps
    }
}

impl VitalsSource for SystemVitals {
    fn vitals(&self) -> NodeVitals {
        // `busy` drives the clamp decision, and it is the agent's call rather
        // than the client's: an idle part at a low clock is normal, the same
        // clock under load is the failure that hides for weeks.
        let device = crate::telemetry::sample_device(self.runner.as_ref(), &self.caps, false);
        let disk = free_bytes(&self.disk_path);
        let docker_ok = self
            .runner
            .run(&docker_probe_argv())
            .is_ok_and(|o| o.success());
        vitals_from_device(
            &device,
            disk,
            docker_ok,
            self.started.elapsed().as_secs(),
            self.caps.sm_clock_healthy_mhz,
        )
    }
}

/// The one way this project asks whether the container runtime is answering.
///
/// `docker info` exposes `.ServerVersion`; `docker version` does not — its
/// field is `.Server.Version`, and asking `version` for `.ServerVersion` exits
/// non-zero with a template error on Docker 29. That is a silent way to report
/// a healthy daemon as unreachable, so there is exactly one definition of the
/// probe and both callers use it.
#[must_use]
pub fn docker_probe_argv() -> Vec<String> {
    vec![
        "docker".to_owned(),
        "info".to_owned(),
        "--format".to_owned(),
        "{{.ServerVersion}}".to_owned(),
    ]
}

/// Free bytes on the filesystem holding `path`, when it can be determined.
///
/// A full model cache is a leading cause of launch failure, so this is worth
/// reporting even though it needs a platform call.
fn free_bytes(path: &std::path::Path) -> Option<f64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        // The cache directory may not exist yet on a fresh install, and statvfs
        // on a missing path fails. What matters is the filesystem that WOULD
        // hold it, so walk up to the nearest ancestor that does exist.
        let mut probe = path;
        while !probe.exists() {
            match probe.parent() {
                Some(parent) => probe = parent,
                None => return None,
            }
        }
        let c = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c` is a valid NUL-terminated path and `stat` is written only
        // by the call, which reports success before we read it.
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) } != 0 {
            return None;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "free space is displayed in gigabytes"
        )]
        Some(stat.f_bavail as f64 * stat.f_frsize as f64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
