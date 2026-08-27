// SPDX-License-Identifier: AGPL-3.0-only

//! Reads links out of `ifconfig` and `networksetup` on macOS.
//!
//! The macOS twin of [`super::linux`], split the same way: everything that
//! decides anything is a pure function over recorded command output, and the
//! impure half is two process spawns at the bottom. Here the split is not just
//! hygiene — there is no Mac in CI or on the dev boxes, so the parsers are the
//! only part of this backend a test can ever hold still.
//!
//! Two stock commands rather than `SystemConfiguration` bindings on purpose:
//! both ship with every macOS, need no privileges, and parsing two stable text
//! formats costs less than carrying an objc dependency for one call at agent
//! startup. Before this backend existed the Linux provider ran on macOS,
//! found no `/sys/class/net`, and the agent enumerated zero interfaces — the
//! MacBook showed no addresses, was undiscoverable, and minted join
//! invitations with an empty command bar.

use super::{ARPHRD_LOOPBACK, FabricProvider, RawIface};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::process::Command;

#[cfg(test)]
mod tests;

/// Reads this machine's links from `ifconfig` and `networksetup`.
#[derive(Debug, Clone, Default)]
pub struct MacFabric;

impl MacFabric {
    /// A provider for the running machine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Which hardware port name the system gives each device.
///
/// `networksetup -listallhardwareports` is the only stock source that says
/// en0 is "Wi-Fi": `ifconfig` presents a wireless link as plain ethernet, and
/// classifying by that would rank Wi-Fi as wired — which
/// `usable_for_cluster()` then offers to a collective.
fn ports_by_device(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut port = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port: ") {
            port = Some(p.trim());
        } else if let Some(dev) = line.strip_prefix("Device: ")
            && let Some(p) = port
        {
            out.insert(dev.trim().to_owned(), p.to_owned());
        }
    }
    out
}

/// Whether a hardware port name means a wireless link.
///
/// "Wi-Fi" on anything current; "AirPort" is what the same port was called on
/// older systems, and both spellings still appear in the wild.
fn is_wireless_port(port: &str) -> bool {
    port == "Wi-Fi" || port.contains("AirPort")
}

/// Convert a `0xffffff00`-style netmask to a prefix length.
///
/// macOS prints netmasks as hex words, never as prefix lengths. Only a
/// contiguous mask has a prefix; anything else — non-contiguous, short, or
/// unparseable — is `None`, and the caller emits a bare address, which
/// [`super::split_prefix`] reports as the honest "unknown subnet" zero rather
/// than a guessed /24.
fn prefix_from_hex_netmask(token: &str) -> Option<u8> {
    let mask = u32::from_str_radix(token.strip_prefix("0x")?, 16).ok()?;
    let ones = mask.leading_ones();
    if mask.trailing_zeros() < 32 - ones {
        return None;
    }
    u8::try_from(ones).ok()
}

/// The comma-separated flag names inside `flags=8863<UP,BROADCAST,...>`.
fn flags_of(header_rest: &str) -> impl Iterator<Item = &str> {
    header_rest
        .split_once('<')
        .and_then(|(_, rest)| rest.split_once('>'))
        .map_or("", |(flags, _)| flags)
        .split(',')
}

/// Every interface in an `ifconfig -a` listing, joined with the hardware port
/// table.
///
/// Pure, so the whole backend short of two process spawns is testable from a
/// Linux box against output recorded on a real Mac.
#[must_use]
pub fn interfaces_from(ifconfig: &str, hardware_ports: &str) -> Vec<RawIface> {
    let ports = ports_by_device(hardware_ports);
    let mut out: Vec<RawIface> = Vec::new();
    for line in ifconfig.lines() {
        if !line.starts_with([' ', '\t']) {
            // `en0: flags=8863<UP,...> mtu 1500` opens a block; everything
            // indented under it belongs to that interface.
            let Some((name, rest)) = line.split_once(':') else {
                continue;
            };
            let loopback = flags_of(rest).any(|f| f == "LOOPBACK");
            out.push(RawIface {
                name: name.to_owned(),
                addrs: Vec::new(),
                // `status:` appears further down the block, when the driver
                // reports one at all. Until it says "active" the link must not
                // present as UP — the same rule as the Linux provider's
                // unreadable `carrier` file.
                carrier: false,
                // macOS has no portable negotiated-speed source. None is an
                // absence; any number here would be an invention.
                speed_mbps: None,
                // macOS has no ARPHRD table. The LOOPBACK flag is the one
                // unambiguous fact the header carries, and 772 is the value
                // `classify()` understands for it; everything else stays 0,
                // unknown, because inventing an ethernet type number would be
                // a guess.
                arp_type: if loopback { ARPHRD_LOOPBACK } else { 0 },
                wireless: ports.get(name).is_some_and(|p| is_wireless_port(p)),
                rdma: false,
            });
            continue;
        }
        let Some(current) = out.last_mut() else {
            continue;
        };
        let mut fields = line.split_whitespace();
        match fields.next() {
            // `inet 192.168.1.23 netmask 0xffffff00 broadcast ...`. inet6 is
            // skipped with everything else: `RawIface::addrs` is IPv4 by
            // contract, and that is also what keeps fe80:: link-local — which
            // no peer can dial without a zone it does not have — out of the
            // reachable list.
            Some("inet") => {
                if let Some(host) = fields.next() {
                    let prefix = fields
                        .find(|f| *f == "netmask")
                        .and_then(|_| fields.next())
                        .and_then(prefix_from_hex_netmask);
                    current.addrs.push(match prefix {
                        Some(p) => format!("{host}/{p}"),
                        None => host.to_owned(),
                    });
                }
            }
            Some("status:") => current.carrier = fields.next() == Some("active"),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

impl FabricProvider for MacFabric {
    fn raw_interfaces(&self) -> Result<Vec<RawIface>> {
        let ifconfig = Command::new("ifconfig")
            .arg("-a")
            .output()
            .context("running `ifconfig -a`")?;
        // Not degraded to an empty port table on failure: without it every
        // wireless link classifies as wired ethernet, which the cluster
        // planner would then treat as a fabric. Reporting that the system
        // could not be read beats reporting that Wi-Fi is a wire.
        let ports = Command::new("networksetup")
            .args(["-listallhardwareports"])
            .output()
            .context("running `networksetup -listallhardwareports`")?;
        Ok(interfaces_from(
            &String::from_utf8_lossy(&ifconfig.stdout),
            &String::from_utf8_lossy(&ports.stdout),
        ))
    }
}
