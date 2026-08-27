// SPDX-License-Identifier: AGPL-3.0-only

//! How a control verb reaches another machine.
//!
//! Split from `fleet.rs` (at the 500-line cap) along the routing seam: this
//! file answers "where would a control request for that node actually go",
//! from THIS agent's own state and nothing else. Both users of that answer —
//! the origin's relay driver and the listing's `reached_via` — resolve
//! through the same functions here and in [`super::vouched`], so the UI never
//! displays a route the router would not take (SSOT).
//!
//! Nothing in this file reads an address out of a frame or a digest. A
//! vouched address is display data; control reaches a vouched node ONLY
//! through its voucher, and a pinned node only at an address this agent's own
//! reports or pin store hold. That is what keeps a relay — and this agent —
//! from being used as an arbitrary-address proxy.

use super::LocalFleet;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::AgentError;
use std::net::SocketAddr;

/// Where a control request for a target would go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlRoute {
    /// The target is pinned: dial it ourselves and send the terminal frame.
    Direct {
        /// Where to dial it, from our own state.
        addr: SocketAddr,
    },
    /// The target is only vouched for: ask its voucher to forward one hop.
    Via {
        /// The voucher chosen by [`LocalFleet::choose_voucher`].
        relay: NodeId,
        /// Where to dial the RELAY, from our own state. Never the target.
        addr: SocketAddr,
    },
}

impl LocalFleet {
    /// The best address this agent itself knows for a pinned peer.
    ///
    /// From our live report first — `max_by_key((class.rank(), speed_mbps))`,
    /// the same selection `cluster.rs` and `rendezvous.rs` use, which is what
    /// puts the fabric ahead of the LAN — falling back to the pin's
    /// remembered address. A frame, a digest, or any other machine's word
    /// contributes zero bytes to this answer.
    #[must_use]
    pub fn control_address(&self, node: NodeId) -> Option<String> {
        let reported = self.reports.lock().ok().and_then(|reports| {
            let (report, _) = reports.get(&node)?;
            report
                .addresses
                .iter()
                .max_by_key(|a| (a.class.rank(), a.speed_mbps.unwrap_or(0)))
                .map(|a| a.addr.clone())
        });
        reported.or_else(|| {
            self.pins
                .load()
                .ok()
                .and_then(|pins| pins.get(&node).and_then(|pin| pin.last_address.clone()))
        })
    }

    /// The highest peer-protocol version a peer has advertised over the
    /// authenticated channel this session, when it has been heard from.
    #[must_use]
    pub fn peer_version_max(&self, node: NodeId) -> Option<u32> {
        self.reports
            .lock()
            .ok()
            .and_then(|r| r.get(&node).map(|(report, _)| report.peer_version_max))
    }

    /// Plan the route a control request for `target` would take (rules
    /// O2–O5), without dialling anything.
    ///
    /// # Errors
    /// `NotRoutable`, naming what is missing: not pinned and not vouched, a
    /// voucher that is currently unreachable, a pinned peer with no known
    /// address, or a next hop that has not advertised control support (a v2
    /// frame at a v1 build is silently dropped, so it is refused here, typed,
    /// instead of sent).
    pub fn plan_control_route(
        &self,
        target: NodeId,
        port: u16,
    ) -> Result<ControlRoute, AgentError> {
        // Fail closed: an unreadable pin store must not downgrade a pinned
        // target to "unpinned" and route it through a voucher instead.
        let pins = self.pins.load().map_err(|e| AgentError::NotRoutable {
            node: target,
            reason: format!("this agent's pin store could not be read: {e:#}"),
        })?;

        if pins.contains_key(&target) {
            // O2 — pinned: direct dial, terminal frame.
            self.require_control_capable(target, target)?;
            let addr = self
                .control_address(target)
                .and_then(|a| to_socket(&a, port))
                .ok_or_else(|| AgentError::NotRoutable {
                    node: target,
                    reason: "it is paired with this machine but has no known address yet; \
                             wait for it to be seen, or re-add it with `atlasctl peer add`"
                        .to_owned(),
                })?;
            return Ok(ControlRoute::Direct { addr });
        }

        // O3 — vouched: forward through the chosen voucher, never a dial to
        // any address a digest carried.
        if let Some(relay) = self.choose_voucher(target) {
            self.require_control_capable(target, relay)?;
            let addr = self
                .control_address(relay)
                .and_then(|a| to_socket(&a, port))
                .ok_or_else(|| AgentError::NotRoutable {
                    node: target,
                    reason: format!(
                        "{}, which vouches for it, has no known address",
                        relay.short()
                    ),
                })?;
            return Ok(ControlRoute::Via { relay, addr });
        }

        // O4 — no route. Say WHICH thing is missing: "pair with it" and
        // "wake its voucher" send the operator to different machines.
        let reason = match self.vouch_of(target) {
            Some((voucher, _)) => format!(
                "not paired with this machine, and {}, which vouches for it, \
                 is not answering",
                voucher.short()
            ),
            None => "not paired with this machine, and no reachable peer vouches for it; \
                     pair with it directly"
                .to_owned(),
        };
        Err(AgentError::NotRoutable {
            node: target,
            reason,
        })
    }

    /// O5 — refuse, by name and version, a next hop that has not advertised
    /// control support. `hop` is the machine that would be dialled; `target`
    /// names the request so the refusal answers the right question.
    fn require_control_capable(&self, target: NodeId, hop: NodeId) -> Result<(), AgentError> {
        match self.peer_version_max(hop) {
            Some(v) if v < 2 => Err(AgentError::NotRoutable {
                node: target,
                reason: format!(
                    "{} speaks peer protocol {v}, which has no control frames; upgrade it",
                    hop.short()
                ),
            }),
            // No report yet is not a refusal: the dial helper re-checks the
            // hello of the actual connection, so an old build is still caught
            // before any control frame is written at it.
            _ => Ok(()),
        }
    }
}

/// Attach the peer port to a stored address string.
///
/// Parsed as an IP and joined structurally, never formatted as "{addr}:{port}"
/// — an IPv6 literal needs brackets, and the formatted form silently failed to
/// parse for every IPv6 peer once already (see the poll loop).
fn to_socket(addr: &str, port: u16) -> Option<SocketAddr> {
    addr.parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, port))
}
