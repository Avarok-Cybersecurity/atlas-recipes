// SPDX-License-Identifier: AGPL-3.0-only

//! What links this machine has, and which of them a collective may use.
//!
//! Split deliberately into two halves. [`classify`] and [`reachable_addresses`]
//! are pure functions over [`RawIface`] values, so the whole selection policy
//! is unit-testable against a recorded interface table with no hardware, no
//! root, and no network. Reading the system to produce those values is the
//! only impure part, and it lives behind [`FabricProvider`].
//!
//! The policy exists because picking naively goes wrong in a way that is
//! invisible. A DGX Spark presents, in kernel order: two docker bridges, a
//! `dummy0` carrying the documented head IP, wifi, and four RoCE ports on
//! point-to-point /30s. Taking the first address with a carrier gives you a
//! docker bridge; taking the first routable one gives you wifi. Either runs the
//! collective over the wrong link and costs multiples of throughput while every
//! correctness check still passes.

use anyhow::Result;
use atlasctl_protocol::fleet::{LinkClass, NodeAddress};

pub mod linux;
// Compiled for every test build, not only on macOS, because no Mac exists in
// CI or on the dev boxes: the recorded-output parsers are the only part of the
// backend a test can ever exercise, and gating them on `target_os` would leave
// them tested nowhere. A Linux release build still never contains this module.
#[cfg(any(test, target_os = "macos"))]
pub mod macos;

#[cfg(test)]
mod tests;

/// Facts about one interface, as read from the system.
///
/// Deliberately dumb: no judgement is applied here, so a test can state exactly
/// what the kernel said and assert what the policy makes of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawIface {
    /// Interface name.
    pub name: String,
    /// IPv4 addresses bound to it, each as `host/prefix`.
    ///
    /// The prefix is kept because it is the only thing that says which of four
    /// point-to-point RoCE links reaches a given peer.
    pub addrs: Vec<String>,
    /// Whether the kernel reports a carrier.
    pub carrier: bool,
    /// Negotiated speed in Mb/s, when the kernel reports one.
    pub speed_mbps: Option<u32>,
    /// ARPHRD type, 772 being loopback.
    pub arp_type: u32,
    /// Whether the interface has a wireless directory.
    pub wireless: bool,
    /// Whether an RDMA device is bound to it.
    pub rdma: bool,
}

/// ARPHRD_LOOPBACK.
const ARPHRD_LOOPBACK: u32 = 772;

/// Interface name prefixes that are always software constructs.
///
/// `dummy` earns its place from experience rather than theory: the documented
/// "head IP" of this cluster, 10.10.10.1, lives on a `dummy0`, so it looks like
/// the most authoritative address on the box while being reachable from
/// precisely nowhere.
const VIRTUAL_PREFIXES: [&str; 7] = ["docker", "br-", "veth", "virbr", "dummy", "tun", "tap"];

/// Decide what kind of link an interface is.
///
/// Pure. Order matters: loopback first because it is unambiguous, then the
/// software constructs by name, then RDMA, then the physical classes.
#[must_use]
pub fn classify(raw: &RawIface) -> LinkClass {
    if raw.arp_type == ARPHRD_LOOPBACK || raw.name == "lo" {
        return LinkClass::Loopback;
    }
    if VIRTUAL_PREFIXES.iter().any(|p| raw.name.starts_with(p)) {
        return LinkClass::Virtual;
    }
    if raw.rdma {
        // An RDMA-backed ethernet port is RoCE. True InfiniBand is reported by
        // the provider setting `speed_mbps` from an IB port rather than a NIC,
        // which we cannot distinguish from the interface alone — so RoCE is the
        // honest answer here and the provider may refine it.
        return LinkClass::Roce;
    }
    if raw.wireless {
        return LinkClass::Wireless;
    }
    LinkClass::Ethernet
}

/// Split `10.10.10.9/30` into its host and prefix.
///
/// A bare address means an unknown subnet, reported as `0` rather than guessed:
/// claiming `/32` would say "shares a link with nothing", and claiming `/24`
/// would say the opposite. Both are inventions; zero is the truth.
fn split_prefix(raw: &str) -> (String, u8) {
    match raw.split_once('/') {
        Some((host, len)) => (host.to_owned(), len.parse().unwrap_or(0)),
        None => (raw.to_owned(), 0),
    }
}

/// Turn a raw interface table into the addresses a peer may be reached on.
///
/// Drops anything with no address, no carrier, or a link that is reachable only
/// from this machine (loopback, and virtual interfaces like a docker bridge),
/// then sorts best-link-first.
///
/// # Why this is not filtered for cluster use
///
/// It used to be, through `usable_for_cluster()` — the question "can this link
/// carry a collective?", which answers NO for wireless. That is the right
/// answer to that question and the wrong filter to apply HERE, because this
/// list is the single source for four different consumers: the mDNS beacon, the
/// node descriptor the browser renders, the addresses offered in a join
/// invitation, and the cluster planner.
///
/// Only the last one wants the collective question. Applying it to all four
/// meant a laptop on Wi-Fi enumerated its interfaces, dropped every one of
/// them here, and went on to advertise no address at all — so it was
/// undiscoverable, showed the operator no addresses, and minted join
/// invitations with nowhere to dial. That laptop is exactly the machine the
/// invitation exists for: it cannot run models, so it invites one that can.
///
/// The collective question is asked where it belongs, at each cluster use site
/// — `NodeDescriptor::preferred_address`, `rendezvous::best_reachable`, and the
/// shared-subnet search in `cluster.rs` all re-filter on `usable_for_cluster`.
/// Reporting the truth once and narrowing per question is the only arrangement
/// where a new consumer cannot silently inherit the wrong filter.
#[must_use]
pub fn reachable_addresses(raws: &[RawIface]) -> Vec<NodeAddress> {
    let mut out: Vec<NodeAddress> = raws
        .iter()
        .filter(|r| r.carrier)
        .flat_map(|r| {
            let class = classify(r);
            r.addrs.iter().map(move |raw| {
                let (host, prefix) = split_prefix(raw);
                NodeAddress {
                    iface: r.name.clone(),
                    addr: host,
                    class,
                    speed_mbps: r.speed_mbps,
                    rdma: r.rdma,
                    prefix_len: prefix,
                }
            })
        })
        .filter(|a| a.class.usable_for_control())
        .collect();

    // Best link first, then fastest, then by name so the order is stable across
    // runs — an unstable node list makes a UI flicker for no reason.
    out.sort_by(|a, b| {
        b.class
            .rank()
            .cmp(&a.class.rank())
            .then(b.speed_mbps.unwrap_or(0).cmp(&a.speed_mbps.unwrap_or(0)))
            .then(a.iface.cmp(&b.iface))
    });
    out
}

/// Reads the machine's links.
///
/// Injected rather than called directly so the cluster path can be exercised
/// against a recorded table — including the awkward real one — with no
/// hardware present.
pub trait FabricProvider: Send + Sync {
    /// Every interface this machine has, unfiltered.
    ///
    /// # Errors
    /// If the system cannot be read at all.
    fn raw_interfaces(&self) -> Result<Vec<RawIface>>;

    /// Addresses a peer may be reached on, best link first.
    ///
    /// # Errors
    /// If the system cannot be read at all.
    fn addresses(&self) -> Result<Vec<NodeAddress>> {
        Ok(reachable_addresses(&self.raw_interfaces()?))
    }
}

/// A fabric provider backed by a fixed table.
///
/// Not test-only scaffolding hiding in production: `--client` mode on a machine
/// whose links are irrelevant, and the `atlasctl doctor` dry-run path, both want
/// a provider that answers without touching the system.
#[derive(Debug, Clone, Default)]
pub struct StaticFabric {
    /// The table to answer with.
    pub ifaces: Vec<RawIface>,
}

impl FabricProvider for StaticFabric {
    fn raw_interfaces(&self) -> Result<Vec<RawIface>> {
        Ok(self.ifaces.clone())
    }
}
