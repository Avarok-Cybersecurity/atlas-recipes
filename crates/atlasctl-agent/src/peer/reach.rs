// SPDX-License-Identifier: AGPL-3.0-only

//! Did we ever get to talk to that machine?
//!
//! An inviting machine offers several addresses because it cannot know which
//! of its networks the new machine shares, and the joiner tries them in turn.
//! Whether trying the next one is FREE depends entirely on how the last one
//! failed, and the two cases look identical from a distance:
//!
//! - nothing answered — wrong network, firewall, box asleep. The code was
//!   never presented, so the next address costs nothing.
//! - the machine answered and said no. Every address in the list is the same
//!   machine, so this already spent one of the code's [`crate::pairing::
//!   MAX_ATTEMPTS`] tries. Marching through the rest spends the remainder and
//!   locks the operator out — on their FIRST mistyped code, before they have
//!   had one real go.
//!
//! With three attempts and a DGX that advertises three addresses, not making
//! this distinction turns a single typo into a lockout.

use std::io::ErrorKind;

/// Whether `err` means no peer ever answered, so another address may be tried
/// without spending an attempt.
///
/// Errs toward `false`: an unrecognised failure stops the walk. Giving up one
/// address early is a worse message; giving up an attempt is a lockout.
#[must_use]
pub fn never_reached(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            matches!(
                io.kind(),
                ErrorKind::ConnectionRefused
                    | ErrorKind::TimedOut
                    | ErrorKind::HostUnreachable
                    | ErrorKind::NetworkUnreachable
                    | ErrorKind::AddrNotAvailable
                    | ErrorKind::NotFound
            )
        })
    })
}

/// Try each address in turn, stopping the moment a machine answers — whether
/// it says yes or no.
///
/// Both callers walk the same list under the same rule — `fleet::listing`
/// pairing a discovered machine, and `agent install --join` joining someone
/// else's fleet — so the rule lives once. Errors accumulate: reporting only
/// the last names whichever address sorted last, usually the least
/// interesting failure, and hides that several links were tried.
///
/// # Errors
/// When no address produced a success, with every reason, in order.
pub fn walk<T, F>(
    addrs: &[std::net::SocketAddr],
    mut dial: F,
) -> anyhow::Result<(std::net::SocketAddr, T)>
where
    F: FnMut(std::net::SocketAddr) -> anyhow::Result<T>,
{
    let mut why: Vec<String> = Vec::new();
    for addr in addrs {
        match dial(*addr) {
            Ok(v) => return Ok((*addr, v)),
            Err(e) => {
                let keep_going = never_reached(&e);
                why.push(format!("{addr}: {e:#}"));
                if !keep_going {
                    break;
                }
            }
        }
    }
    anyhow::bail!("{}", why.join("; "))
}

/// Where to dial one peer: its address, and the port IT advertised.
///
/// Two rules, both of which were learned the hard way and neither of which
/// had a test while they lived inside the poll loop:
///
/// 1. Parse the IP and attach the port structurally. Formatting
///    `"{addr}:{port}"` and parsing that back needs an IPv6 literal in
///    brackets, so it failed for EVERY IPv6 peer — and the caller's `continue`
///    turned that into silence: the node stayed in the fleet, was never
///    polled, and aged into "stale" forever with no error anywhere.
/// 2. Prefer the port the PEER announced. `None` means it is not currently
///    announcing, and only then does this agent's own port stand in. Using
///    ours unconditionally was correct only while every agent bound the same
///    one — the assumption that makes a per-machine port unaddable.
///
/// `None` when the address does not parse as an IP, which the caller skips.
#[must_use]
pub fn dial_socket(
    addr: &str,
    advertised: Option<u16>,
    fallback: u16,
) -> Option<std::net::SocketAddr> {
    addr.parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, advertised.unwrap_or(fallback)))
}

#[cfg(test)]
mod tests {
    use super::never_reached;
    use anyhow::anyhow;
    use std::io::{Error, ErrorKind};

    fn io(kind: ErrorKind) -> anyhow::Error {
        anyhow::Error::new(Error::new(kind, "boom"))
    }

    #[test]
    fn the_ways_a_wrong_network_fails_all_allow_the_next_address() {
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::TimedOut,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
        ] {
            assert!(never_reached(&io(kind)), "{kind:?} means nobody answered");
        }
    }

    #[test]
    fn a_transport_failure_is_still_recognised_under_context() {
        // The real call sites wrap with `.context(…)`, so the io::Error is
        // never the outermost cause; looking only at the top would report
        // every failure as a refusal and stop after one address.
        let e = io(ErrorKind::ConnectionRefused).context("dialling 10.10.10.9:34334");
        assert!(never_reached(&e));
    }

    #[test]
    fn a_refusal_by_the_far_machine_ends_the_walk() {
        // This is the one that matters: the peer DID answer. Trying the next
        // address spends another of three attempts on the same machine.
        let e = anyhow!("key confirmation failed: the code does not match");
        assert!(
            !never_reached(&e),
            "a machine that answered and refused must not cost a second attempt"
        );
    }

    #[test]
    fn an_unrecognised_failure_stops_rather_than_spending_an_attempt() {
        assert!(!never_reached(&io(ErrorKind::InvalidData)));
    }

    fn a(n: u8) -> std::net::SocketAddr {
        std::net::SocketAddr::from(([10, 10, 10, n], 34334))
    }

    #[test]
    fn the_walk_stops_at_the_first_machine_that_answers_and_refuses() {
        let mut seen = Vec::new();
        let out = super::walk(&[a(9), a(13), a(68)], |addr| -> anyhow::Result<()> {
            seen.push(addr);
            Err(anyhow!("key confirmation failed"))
        });
        assert!(out.is_err());
        assert_eq!(seen, vec![a(9)], "the rest are the same machine");
    }

    #[test]
    fn the_walk_continues_past_every_address_that_never_answers() {
        let mut seen = Vec::new();
        let out = super::walk(&[a(9), a(13), a(68)], |addr| -> anyhow::Result<()> {
            seen.push(addr);
            Err(io(ErrorKind::HostUnreachable))
        });
        let e = out.expect_err("nothing answered").to_string();
        assert_eq!(seen, vec![a(9), a(13), a(68)]);
        for n in [9u8, 13, 68] {
            assert!(
                e.contains(&format!("10.10.10.{n}")),
                "every link tried must be named, not just the last: {e}"
            );
        }
    }

    #[test]
    fn the_address_that_worked_is_the_one_returned() {
        // The caller pins it, so returning the first tried would record a link
        // the machine has just proved it cannot use.
        let (addr, v) = super::walk(&[a(9), a(68)], |addr| {
            if addr == a(68) {
                Ok("paired")
            } else {
                Err(io(ErrorKind::TimedOut))
            }
        })
        .expect("the LAN address answers");
        assert_eq!(addr, a(68));
        assert_eq!(v, "paired");
    }

    #[test]
    fn an_empty_list_is_an_error_rather_than_a_silent_success() {
        let out = super::walk(&[], |_| -> anyhow::Result<()> {
            panic!("nothing to dial must not dial")
        });
        assert!(out.is_err());
    }

    #[test]
    fn the_port_the_peer_announced_wins_over_this_agents_own() {
        let s = super::dial_socket("10.10.10.9", Some(34999), 34334).expect("parses");
        assert_eq!(s.port(), 34999);
    }

    #[test]
    fn a_peer_that_is_not_announcing_falls_back_rather_than_being_skipped() {
        // `None` is "it did not say", not "port zero" and not "unreachable".
        let s = super::dial_socket("10.10.10.9", None, 34334).expect("parses");
        assert_eq!(s.port(), 34334);
    }

    /// The regression this shape exists for: `format!("{addr}:{port}")` then
    /// parsing back needs brackets, so every IPv6 peer produced `None` and the
    /// caller silently skipped it forever.
    #[test]
    fn an_ipv6_peer_is_dialable_at_all() {
        let s = super::dial_socket("fe80::1", Some(34999), 34334).expect("a v6 peer parses");
        assert!(s.is_ipv6());
        assert_eq!(s.port(), 34999);
    }

    #[test]
    fn a_hostname_is_not_an_address_here() {
        // This path takes addresses off a beacon or a pin, never a name; a
        // name would have to be resolved, which is `resolve_manual`'s job.
        assert!(super::dial_socket("spark-256a", None, 34334).is_none());
    }
}
