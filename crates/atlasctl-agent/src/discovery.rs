// SPDX-License-Identifier: AGPL-3.0-only

//! Finding other agents on the LAN.
//!
//! **Discovery grants zero authority.** A beacon draws a grey node on someone's
//! screen and can cause nothing else to happen: it cannot make this agent
//! connect anywhere privileged, cannot enter the pin store, and cannot launch
//! anything. Trust is established only by the pairing ceremony, which requires
//! a code read off the target machine by a human. That separation is why it is
//! safe to advertise by default.
//!
//! Everything in a beacon is therefore **untrusted input**, and is treated as
//! such at the boundary: names go through [`DisplayName`], and the record
//! deliberately carries no agent version (no scanning for a known-vulnerable
//! release), no recipe inventory, no running-job detail, and not the browser
//! control port.
//!
//! `atlasctl peer add <host>` is a first-class path rather than a fallback.
//! Enterprise wifi does client isolation, plenty of switches filter multicast,
//! and the DGX RoCE links here are point-to-point /30s where multicast reaches
//! exactly one peer anyway.

use anyhow::{Context, Result};
use atlasctl_protocol::fleet::{DisplayName, NodeId};
use std::net::IpAddr;
use std::sync::mpsc::Receiver;

pub mod mdns;

#[cfg(test)]
mod tests;

/// The DNS-SD service type agents advertise under.
pub const SERVICE_TYPE: &str = "_atlasctl._tcp.local.";

/// TXT key for the node fingerprint.
const TXT_ID: &str = "id";
/// TXT key for the display name.
const TXT_NAME: &str = "name";
/// TXT key for whether the node can run a model.
const TXT_CAN_LAUNCH: &str = "cl";
/// TXT key for the coarse accelerator tag.
const TXT_ACCEL: &str = "gpu";

/// What a node says about itself on the wire.
///
/// Small on purpose. Anything added here is published to every device on the
/// network, unauthenticated, forever.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beacon {
    /// Fingerprint of the advertising node's key.
    pub id: NodeId,
    /// Hostname, already sanitised.
    pub name: DisplayName,
    /// Port the *peer* channel listens on. Never the browser port.
    pub peer_port: u16,
    /// Addresses the node was seen at.
    pub addresses: Vec<IpAddr>,
    /// Whether it claims it can run a model. A claim, not a permission.
    pub can_launch: bool,
    /// Coarse accelerator tag, for display before pairing.
    pub accelerator: String,
}

impl Beacon {
    /// The TXT properties for this beacon.
    #[must_use]
    pub fn txt_properties(&self) -> Vec<(String, String)> {
        vec![
            (TXT_ID.to_owned(), self.id.to_string()),
            (TXT_NAME.to_owned(), self.name.as_str().to_owned()),
            (
                TXT_CAN_LAUNCH.to_owned(),
                if self.can_launch { "1" } else { "0" }.to_owned(),
            ),
            (TXT_ACCEL.to_owned(), self.accelerator.clone()),
        ]
    }

    /// Rebuild a beacon from TXT properties and observed addresses.
    ///
    /// Returns `None` rather than erroring when the record is not one of ours
    /// or carries no usable identity — a malformed beacon is noise on a shared
    /// network, not a fault in this program.
    #[must_use]
    pub fn from_txt(
        props: &[(String, String)],
        addresses: Vec<IpAddr>,
        peer_port: u16,
    ) -> Option<Self> {
        let get = |k: &str| {
            props
                .iter()
                .find(|(pk, _)| pk == k)
                .map(|(_, v)| v.as_str())
        };
        // No parseable fingerprint means no identity, and identity is the only
        // field that matters. Everything else has a safe default.
        let id = NodeId::parse(get(TXT_ID)?).ok()?;
        Some(Self {
            id,
            name: DisplayName::new(get(TXT_NAME).unwrap_or("unnamed")),
            peer_port,
            addresses,
            can_launch: get(TXT_CAN_LAUNCH) == Some("1"),
            // Length-capped through DisplayName for the same reason the name is.
            // Length-capped like every other beacon string, but an absent tag
            // stays absent: DisplayName's "unnamed" fallback is right for a
            // hostname and wrong for a hardware label nobody supplied.
            accelerator: match get(TXT_ACCEL).map(str::trim).filter(|s| !s.is_empty()) {
                Some(tag) => DisplayName::new(tag).as_str().to_owned(),
                None => String::new(),
            },
        })
    }
}

/// Something discovery noticed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A node appeared, or refreshed its record.
    Found(Box<Beacon>),
    /// A node's record went away.
    ///
    /// Acted on with hysteresis by the caller: a single missed refresh on a
    /// busy wifi network must not make a node blink out of the UI.
    Lost(NodeId),
}

/// Publishes this node's beacon.
pub trait Advertiser: Send + Sync {
    /// Start advertising.
    ///
    /// # Errors
    /// If the responder cannot be started or the record cannot be registered.
    fn advertise(&self, beacon: &Beacon) -> Result<()>;

    /// Stop advertising.
    ///
    /// # Errors
    /// If the record cannot be withdrawn.
    fn withdraw(&self) -> Result<()>;
}

/// Watches for other nodes.
pub trait DiscoveryBrowser: Send + Sync {
    /// Begin browsing. Events arrive on the returned channel until it is dropped.
    ///
    /// # Errors
    /// If the responder cannot be started.
    fn browse(&self) -> Result<Receiver<DiscoveryEvent>>;
}

/// Discovery that does nothing.
///
/// The hardened preset (`advertise = false`, `browse = false`) and `--client`
/// mode on a network where multicast is filtered both want a real object that
/// answers "nothing here" rather than an `Option` threaded through every call
/// site.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDiscovery;

impl Advertiser for NoDiscovery {
    fn advertise(&self, _beacon: &Beacon) -> Result<()> {
        Ok(())
    }

    fn withdraw(&self) -> Result<()> {
        Ok(())
    }
}

impl DiscoveryBrowser for NoDiscovery {
    fn browse(&self) -> Result<Receiver<DiscoveryEvent>> {
        let (tx, rx) = std::sync::mpsc::channel();
        // Dropping the sender closes the channel, so a caller that loops over
        // it terminates immediately instead of blocking forever.
        drop(tx);
        Ok(rx)
    }
}

/// This machine's hostname, or a stable fallback.
///
/// Display only. Two Sparks called `spark-256a` are still two different nodes,
/// because a node is its key.
#[must_use]
pub fn local_display_name() -> DisplayName {
    let raw = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::env::var("HOSTNAME").map_err(std::io::Error::other))
        .unwrap_or_else(|_| "atlas-node".to_owned());
    DisplayName::new(&raw)
}

/// Resolve a `host` or `host:port` the user typed into addresses to try.
///
/// # Errors
/// If the name does not resolve.
pub fn resolve_manual(target: &str, default_port: u16) -> Result<Vec<std::net::SocketAddr>> {
    use std::net::ToSocketAddrs;
    let with_port = if target.contains(':') {
        target.to_owned()
    } else {
        format!("{target}:{default_port}")
    };
    Ok(with_port
        .to_socket_addrs()
        .with_context(|| format!("resolving {target}"))?
        .collect())
}
