// SPDX-License-Identifier: AGPL-3.0-only

//! mDNS-SD discovery, via `mdns-sd`.
//!
//! Pure Rust rather than binding Avahi or Bonjour: linking a C daemon's client
//! library would break the static musl build that makes the single-binary
//! install work, and adding a C dependency to a project whose reason for
//! existing is a supply-chain compromise is the wrong trade.
//!
//! Everything that arrives here is untrusted. Parsing happens in
//! [`super::Beacon::from_txt`], which drops records it cannot make sense of
//! rather than propagating an error: on a shared network, malformed records are
//! ambient noise.

use super::{Advertiser, Beacon, DiscoveryBrowser, DiscoveryEvent, SERVICE_TYPE};
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::net::IpAddr;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;

/// mDNS discovery backed by a `ServiceDaemon`.
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    /// Full name of the record we registered, so it can be withdrawn.
    registered: Mutex<Option<String>>,
}

impl std::fmt::Debug for MdnsDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdnsDiscovery").finish_non_exhaustive()
    }
}

impl MdnsDiscovery {
    /// Start a responder.
    ///
    /// # Errors
    /// If the multicast socket cannot be opened, which is common enough on
    /// locked-down networks that the caller is expected to fall back to manual
    /// peer addition rather than treat it as fatal.
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new().context("starting the mDNS responder")?;
        Ok(Self {
            daemon,
            registered: Mutex::new(None),
        })
    }
}

impl Advertiser for MdnsDiscovery {
    fn advertise(&self, beacon: &Beacon) -> Result<()> {
        // The instance name must be unique on the network and must not be
        // something an observer can use to correlate machines beyond what the
        // beacon already says. The fingerprint's first 16 hex characters are
        // already public in the TXT record, so reusing them adds nothing.
        let instance = format!("atlas-{}", &beacon.id.to_string()[..16]);
        let host = format!("{instance}.local.");
        let props = beacon.txt_properties();

        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host,
            beacon.addresses.as_slice(),
            beacon.peer_port,
            &props[..],
        )
        .context("building the service record")?;

        let fullname = info.get_fullname().to_owned();
        self.daemon
            .register(info)
            .context("registering the service record")?;
        *self
            .registered
            .lock()
            .map_err(|_| anyhow::anyhow!("discovery lock poisoned"))? = Some(fullname);
        Ok(())
    }

    fn withdraw(&self) -> Result<()> {
        let mut guard = self
            .registered
            .lock()
            .map_err(|_| anyhow::anyhow!("discovery lock poisoned"))?;
        if let Some(fullname) = guard.take() {
            self.daemon
                .unregister(&fullname)
                .context("withdrawing the service record")?;
        }
        Ok(())
    }
}

impl DiscoveryBrowser for MdnsDiscovery {
    fn browse(&self) -> Result<Receiver<DiscoveryEvent>> {
        let incoming = self
            .daemon
            .browse(SERVICE_TYPE)
            .context("browsing for agents")?;
        let (tx, rx) = std::sync::mpsc::channel();

        // A thread rather than a task: mdns-sd's receiver is synchronous, and
        // this is one long-lived thread per agent, not per peer.
        std::thread::Builder::new()
            .name("atlasctl-mdns".to_owned())
            .spawn(move || {
                while let Ok(event) = incoming.recv() {
                    let out = match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let props: Vec<(String, String)> = info
                                .get_properties()
                                .iter()
                                .map(|p| (p.key().to_owned(), p.val_str().to_owned()))
                                .collect();
                            // mdns-sd yields `ScopedIp`, which may carry a
                            // zone index for link-local v6. The peer channel
                            // dials plain addresses, so the zone is dropped
                            // here rather than threaded through the protocol.
                            let addrs: Vec<IpAddr> = info
                                .get_addresses()
                                .iter()
                                .map(|s| s.to_ip_addr())
                                .collect();
                            Beacon::from_txt(&props, addrs, info.get_port())
                                .map(|b| DiscoveryEvent::Found(Box::new(b)))
                        }
                        // Deliberately not turned into a `Lost`. The record
                        // name carries only half a fingerprint, so it cannot
                        // name a `NodeId` — and more importantly, one missed
                        // refresh on busy wifi is normal. Departure is decided
                        // by the caller's staleness timer, which requires a
                        // node to be absent across two discovery intervals, so
                        // nodes do not blink out of the interface.
                        _ => None,
                    };
                    if let Some(ev) = out
                        && tx.send(ev).is_err()
                    {
                        break;
                    }
                }
            })
            .context("spawning the discovery thread")?;

        Ok(rx)
    }
}
