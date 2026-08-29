// SPDX-License-Identifier: AGPL-3.0-only

//! Reads links out of sysfs and the kernel's address list.
//!
//! The only impure half of [`super`]. Everything it produces is a plain
//! [`RawIface`], so the selection policy stays testable against a recorded
//! table.
//!
//! sysfs rather than a netlink crate on purpose: the four files involved are a
//! stable kernel ABI, reading them needs no privileges, and it keeps a static
//! musl build free of another C dependency. Addresses come from `ip -o -4 addr`
//! because parsing netlink by hand to save one process is not a trade worth
//! making at agent startup.

use super::{FabricProvider, RawIface};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

/// Where the kernel exposes interfaces.
const NET_DIR: &str = "/sys/class/net";
/// Where the kernel exposes RDMA devices.
const IB_DIR: &str = "/sys/class/infiniband";

/// Reads this machine's links from sysfs.
#[derive(Debug, Clone, Default)]
pub struct LinuxFabric;

impl LinuxFabric {
    /// A provider for the running machine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Read a sysfs file and trim it.
fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Which interfaces have an RDMA device bound to them.
///
/// `/sys/class/infiniband/<dev>/device/net/<iface>` is the link back from an
/// RDMA device to the netdev it fronts. On the DGX Sparks this reports
/// `rocep1s0f0 -> enp1s0f0np0`, which is exactly how a RoCE port is told apart
/// from an ordinary NIC that happens to be fast.
fn rdma_backed_interfaces() -> BTreeSet<String> {
    rdma_devices_by_interface().into_keys().collect()
}

/// Which RDMA device backs each network interface.
///
/// The mapping, not just the set. NCCL is told *which* device to use, and on a
/// machine with four RoCE ports the difference between `rocep1s0f0` and
/// `rocep1s0f1` is the difference between a collective that runs and one that
/// times out at `ibv_modify_qp`.
pub fn rdma_devices_by_interface() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(devices) = std::fs::read_dir(IB_DIR) else {
        return out;
    };
    for dev in devices.flatten() {
        let Some(ibdev) = dev.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let net = dev.path().join("device").join("net");
        let Ok(entries) = std::fs::read_dir(net) else {
            continue;
        };
        for e in entries.flatten() {
            if let Some(name) = e.file_name().to_str() {
                out.insert(name.to_owned(), ibdev.clone());
            }
        }
    }
    out
}

/// IPv4 addresses per interface, each as `host/prefix`.
///
/// The prefix is kept. An earlier version dropped it — "a peer address is a
/// host address" — and that is exactly the information needed to tell which of
/// four point-to-point RoCE links reaches a given peer. Without it the head
/// chose a rendezvous address on a link the worker was not attached to, and
/// the cluster hung at the NCCL barrier.
fn addresses_by_interface() -> Result<BTreeMap<String, Vec<String>>> {
    let out = Command::new("ip")
        .args(["-o", "-4", "addr", "show"])
        .output()
        .context("running `ip -o -4 addr show`")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        // `1: lo    inet 127.0.0.1/8 scope host lo\       valid_lft ...`
        let mut fields = line.split_whitespace();
        let Some(iface) = fields.nth(1) else { continue };
        let mut rest = fields;
        let Some(addr) = rest
            .position(|f| f == "inet")
            .and_then(|_| line.split_whitespace().skip_while(|f| *f != "inet").nth(1))
        else {
            continue;
        };
        map.entry(iface.to_owned())
            .or_default()
            .push(addr.to_owned());
    }
    Ok(map)
}

impl FabricProvider for LinuxFabric {
    fn raw_interfaces(&self) -> Result<Vec<RawIface>> {
        // NOT `unwrap_or_default()`. `ip` missing — a slim container without
        // iproute2 — would otherwise give every interface an empty address
        // list, and a caller cannot tell that from a machine that genuinely
        // has no address. It then reports "no usable network link", sending
        // the operator to inspect a network that was never the problem.
        let addrs = addresses_by_interface()?;
        let rdma = rdma_backed_interfaces();

        let entries = std::fs::read_dir(NET_DIR).with_context(|| format!("reading {NET_DIR}"))?;
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let dir = entry.path();

            // A missing or unreadable attribute means "no", never a default
            // that flatters the interface: an unknown carrier must not present
            // as UP, or a down link becomes a candidate address.
            let carrier = read_trimmed(&dir.join("carrier")).as_deref() == Some("1");
            // `speed` is -1 on virtual devices and errors on some drivers.
            let speed_mbps = read_trimmed(&dir.join("speed"))
                .and_then(|s| s.parse::<i64>().ok())
                .and_then(|v| u32::try_from(v).ok());
            let arp_type = read_trimmed(&dir.join("type"))
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let wireless = dir.join("wireless").exists() || dir.join("phy80211").exists();

            out.push(RawIface {
                addrs: addrs.get(&name).cloned().unwrap_or_default(),
                carrier,
                speed_mbps,
                arp_type,
                wireless,
                rdma: rdma.contains(&name),
                name,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}
