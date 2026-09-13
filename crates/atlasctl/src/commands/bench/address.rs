// SPDX-License-Identifier: AGPL-3.0-only

//! Turning `--with-nodes 10.10.10.2,dgx3.local:34334` into sockets.
//!
//! The forms are the ones an operator types: `ip[:port]`, `[v6]:port`,
//! `host.local[:port]`, `dns.name[:port]`. A missing port is the default
//! peer port, never a guess. Resolution goes through the resolver the
//! machine already has (which answers `.local` wherever mDNS is wired into
//! it); when that fails for a `.local` name, a short browse for atlasctl's
//! own service record answers instead, so a laptop without nss-mdns still
//! reaches a node by its name.

use super::exit::BenchError;
use atlasctl_agent::discovery::{DiscoveryBrowser, DiscoveryEvent, resolve_manual};
use atlasctl_agent::peer::DEFAULT_PEER_PORT;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// How long a browse for a `.local` name may take. Three seconds covers two
/// mDNS query intervals; a node that has not answered by then is not on
/// this link.
pub const MDNS_BROWSE: Duration = Duration::from_secs(3);

/// A node address, split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Exactly as typed, for messages.
    pub given: String,
    /// The host part: an IP literal, a `.local` name, or a DNS name.
    pub host: String,
    /// The port, `DEFAULT_PEER_PORT` when the address carried none.
    pub port: u16,
}

impl Target {
    /// Split an address into host and port without touching the network.
    ///
    /// # Errors
    /// `bad_args` for an empty host, a port that is not a number, or an
    /// unbracketed IPv6 literal with a port (ambiguous).
    pub fn parse(given: &str) -> Result<Self, BenchError> {
        let s = given.trim();
        if s.is_empty() {
            return Err(BenchError::bad_args("empty node address"));
        }
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let Some((inner, after)) = rest.split_once(']') else {
                return Err(BenchError::bad_args(format!("{given}: unclosed `[`")));
            };
            match after.strip_prefix(':') {
                Some(p) => (inner.to_owned(), Some(p)),
                None if after.is_empty() => (inner.to_owned(), None),
                None => {
                    return Err(BenchError::bad_args(format!(
                        "{given}: expected `:port` after `]`"
                    )));
                }
            }
        } else if s.parse::<std::net::Ipv6Addr>().is_ok() {
            (s.to_owned(), None)
        } else if s.matches(':').count() > 1 {
            return Err(BenchError::bad_args(format!(
                "{given}: an IPv6 address with a port must be written `[addr]:port`"
            )));
        } else if let Some((h, p)) = s.split_once(':') {
            (h.to_owned(), Some(p))
        } else {
            (s.to_owned(), None)
        };
        if host.is_empty() {
            return Err(BenchError::bad_args(format!("{given}: empty host")));
        }
        let port = match port {
            None => DEFAULT_PEER_PORT,
            Some(p) => p.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| {
                BenchError::bad_args(format!("{given}: port {p:?} is not a port"))
            })?,
        };
        Ok(Self {
            given: given.to_owned(),
            host,
            port,
        })
    }

    /// True for `name.local` / `name.local.`.
    #[must_use]
    pub fn is_mdns_name(&self) -> bool {
        let h = self.host.trim_end_matches('.');
        h.len() > ".local".len() && h.to_ascii_lowercase().ends_with(".local")
    }

    /// The label before `.local`, for matching a beacon's display name.
    #[must_use]
    pub fn mdns_label(&self) -> String {
        let h = self.host.trim_end_matches('.');
        h[..h.len() - ".local".len()].to_ascii_lowercase()
    }

    /// Resolve to socket addresses: the system resolver first, then — for a
    /// `.local` name only — a browse of atlasctl's service records.
    ///
    /// # Errors
    /// `unreachable` when nothing resolves.
    pub fn resolve(
        &self,
        browser: Option<&dyn DiscoveryBrowser>,
    ) -> Result<Vec<SocketAddr>, BenchError> {
        self.resolve_with(
            |hostport, port| resolve_manual(hostport, port).unwrap_or_default(),
            browser,
            MDNS_BROWSE,
        )
    }

    /// [`Target::resolve`] over a supplied system resolver, so the policy —
    /// system first, browse only for `.local`, browse answers with the
    /// beacon's own port — is testable without a network.
    ///
    /// # Errors
    /// `unreachable` when nothing resolves.
    pub fn resolve_with(
        &self,
        system: impl Fn(&str, u16) -> Vec<SocketAddr>,
        browser: Option<&dyn DiscoveryBrowser>,
        browse_budget: Duration,
    ) -> Result<Vec<SocketAddr>, BenchError> {
        let hostport = if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let system = system(&hostport, self.port);
        if !system.is_empty() {
            return Ok(system);
        }
        if self.is_mdns_name()
            && let Some(b) = browser
            && let Some(found) = browse_for(b, &self.mdns_label(), browse_budget)
        {
            return Ok(found);
        }
        Err(BenchError::unreachable(format!(
            "{} does not resolve{}",
            self.given,
            if self.is_mdns_name() {
                " (no DNS answer, and no atlasctl agent by that name on this link)"
            } else {
                ""
            }
        ))
        .at(&self.given))
    }
}

/// Watch discovery for up to `budget` for a beacon named `label`; its
/// advertised addresses with ITS peer port, not ours — the beacon knows
/// where it listens.
fn browse_for(
    browser: &dyn DiscoveryBrowser,
    label: &str,
    budget: Duration,
) -> Option<Vec<SocketAddr>> {
    let rx = browser.browse().ok()?;
    let deadline = Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match rx.recv_timeout(left) {
            Ok(DiscoveryEvent::Found(b)) => {
                if b.name.as_str().to_ascii_lowercase() == label && !b.addresses.is_empty() {
                    return Some(
                        b.addresses
                            .iter()
                            .map(|ip| SocketAddr::new(*ip, b.peer_port))
                            .collect(),
                    );
                }
            }
            Ok(DiscoveryEvent::Lost(_)) => {}
            Err(_) => return None,
        }
    }
}

/// Parse every address up front, so a typo in the third node is reported
/// before the first is dialled.
///
/// # Errors
/// The first `bad_args`, naming the address.
pub fn parse_all(given: &[String]) -> Result<Vec<Target>, BenchError> {
    let mut out = Vec::with_capacity(given.len());
    for g in given {
        let t = Target::parse(g)?;
        if out
            .iter()
            .any(|o: &Target| o.host == t.host && o.port == t.port)
        {
            return Err(BenchError::bad_args(format!("{g} is listed twice")));
        }
        out.push(t);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "address_tests.rs"]
mod address_tests;
