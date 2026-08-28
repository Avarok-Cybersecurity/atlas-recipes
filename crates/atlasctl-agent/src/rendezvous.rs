// SPDX-License-Identifier: AGPL-3.0-only

//! Can this machine actually reach the address it has been told to meet at?
//!
//! A rank that cannot reach the rendezvous does not fail. It waits at the NCCL
//! barrier, retrying, for as long as the collective's timeout allows — and the
//! operator sees two containers running and no server. That happened on the
//! real pair: a DGX Spark carries four RoCE ports on separate point-to-point
//! `/30`s, the head offered its address on one of them, and the worker was
//! attached to a different one. `Connection timed out (os error 110), retrying`
//! every second, forever.
//!
//! So this is checked during prepare, where a refusal is cheap and says why,
//! rather than after commit, where it is a silent hang.
//!
//! Two questions, in order of confidence:
//!
//! 1. **Is it on a subnet I am directly attached to?** Exact, needs no network,
//!    and is the case that matters for point-to-point RoCE.
//! 2. **Does something answer there?** For anything routed, where subnet
//!    arithmetic cannot see the path. A refused connection proves the host is
//!    reachable just as well as an accepted one — better, since nothing is
//!    listening on the rendezvous port until rank 0 starts.

use atlasctl_protocol::fleet::NodeAddress;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// How long to wait for the routed check before calling it unreachable.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Whether `target` sits on a subnet one of `local` is directly attached to.
///
/// Addresses with an unknown prefix (`0`) are skipped rather than guessed at: a
/// beacon reports a host address with no subnet, and treating that as `/32`
/// would say "shares a link with nothing" while `/24` would say the opposite.
#[must_use]
pub fn on_local_subnet(target: &str, local: &[NodeAddress]) -> bool {
    let Ok(t) = target.parse::<Ipv4Addr>() else {
        return false;
    };
    local.iter().any(|a| {
        if a.prefix_len == 0 || a.prefix_len > 32 {
            return false;
        }
        let Ok(host) = a.addr.parse::<Ipv4Addr>() else {
            return false;
        };
        same_network(host, t, a.prefix_len)
    })
}

/// Whether `other` sits inside `host`'s network of the given prefix length.
///
/// Used by the head when choosing a rendezvous address, and by a rank when
/// judging one it has been given — the same arithmetic on both sides, so the
/// two cannot disagree about what "reachable" means.
#[must_use]
pub fn shares_network(host: &str, prefix: u8, other: &str) -> bool {
    match (host.parse::<Ipv4Addr>(), other.parse::<Ipv4Addr>()) {
        (Ok(h), Ok(o)) => same_network(h, o, prefix),
        _ => false,
    }
}

/// Whether two addresses share a network of the given prefix length.
fn same_network(a: Ipv4Addr, b: Ipv4Addr, prefix: u8) -> bool {
    // A /0 would make every address "local", which is never what a fabric
    // table means; callers filter it out, and this is the second line.
    if prefix == 0 || prefix > 32 {
        return false;
    }
    let mask: u32 = if prefix == 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - prefix)
    };
    (u32::from(a) & mask) == (u32::from(b) & mask)
}

/// Asking the network whether a host is there.
///
/// Behind a trait because it is the only I/O in this module, and without it a
/// test of the refusal path silently depends on the machine it runs on: the
/// address that broke the real cluster is one of *this* host's own interfaces,
/// so a live probe answers "reachable" and the test passes for the wrong
/// reason.
pub trait Reachability: Send + Sync {
    /// Whether something answers at this address, well enough to prove the
    /// host is routable.
    fn answers(&self, addr: &str, port: u16) -> bool;
}

/// The real probe, over TCP.
#[derive(Debug, Clone, Copy, Default)]
pub struct TcpProbe;

impl Reachability for TcpProbe {
    fn answers(&self, addr: &str, port: u16) -> bool {
        answers(addr, port)
    }
}

/// Whether a TCP connect gets far enough to prove the host is there.
///
/// **A refused connection counts as reachable.** Nothing listens on the
/// rendezvous port until rank 0 starts, so "refused" is the expected answer
/// from a healthy peer and the one that distinguishes it from a link that goes
/// nowhere. Only a timeout or a routing error means unreachable.
#[must_use]
pub fn answers(target: &str, port: u16) -> bool {
    let Ok(ip) = target.parse::<IpAddr>() else {
        return false;
    };
    let addr = SocketAddr::new(ip, port);
    connect_outcome(addr).is_some_and(is_an_answer)
}

/// A refusal is an ANSWER: something at that address processed the SYN and said
/// no, which is exactly what a healthy peer does before rank 0 binds the
/// rendezvous port. A reset means the same thing. Silence — a timeout, or the
/// stack saying it has no route — is the link-going-nowhere case this probe
/// exists to separate out.
fn is_an_answer(outcome: Result<(), std::io::ErrorKind>) -> bool {
    match outcome {
        Ok(()) => true,
        Err(k) => matches!(
            k,
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset
        ),
    }
}

/// Connect, giving up after [`PROBE_TIMEOUT`]. `None` means the window expired
/// with no answer at all.
#[cfg(not(windows))]
fn connect_outcome(addr: SocketAddr) -> Option<Result<(), std::io::ErrorKind>> {
    match TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) {
        Ok(_) => Some(Ok(())),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => None,
        Err(e) => Some(Err(e.kind())),
    }
}

/// The same, on Windows, where `connect_timeout` cannot express the answer.
///
/// It reports a REFUSED connection as `TimedOut` — measured, not assumed: a
/// port this process had just released came back `Some(TimedOut)` on CI. The
/// cause is that a failed non-blocking connect is signalled through `select`'s
/// exception set, which that path does not consult, so every refusal is
/// indistinguishable from silence and every healthy peer reads as unreachable.
///
/// A blocking connect on its own thread does report the refusal. The thread is
/// abandoned rather than joined when the window expires; it ends by itself when
/// the OS connect gives up, and a probe must not block the caller for the two
/// minutes that can take.
#[cfg(windows)]
fn connect_outcome(addr: SocketAddr) -> Option<Result<(), std::io::ErrorKind>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(TcpStream::connect(addr).map(|_| ()).map_err(|e| e.kind()));
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok()
}

/// The best address among `candidates` that this machine can actually reach.
///
/// "Best" means the same thing it means everywhere else — highest link class,
/// then fastest — but only among addresses on a subnet `local` is attached to.
/// Falls back to the plain best when nothing is demonstrably shared, because a
/// peer that has not reported its subnets should be dialled optimistically
/// rather than declared unreachable.
///
/// This exists because "the peer's best address" and "the peer's best address
/// *from here*" are different questions, and only the second one can be
/// dialled. A DGX Spark answers on several point-to-point links; picking by
/// class alone returns whichever sorts last among equals, which is a coin flip.
#[must_use]
pub fn best_reachable<'a>(
    candidates: &'a [NodeAddress],
    local: &[NodeAddress],
) -> Option<&'a NodeAddress> {
    let usable = || candidates.iter().filter(|a| a.class.usable_for_cluster());
    usable()
        .filter(|a| on_local_subnet(&a.addr, local))
        .max_by_key(|a| (a.class.rank(), a.speed_mbps.unwrap_or(0)))
        .or_else(|| usable().max_by_key(|a| (a.class.rank(), a.speed_mbps.unwrap_or(0))))
}

/// Why a rendezvous address was rejected, phrased for someone who has to fix it.
#[must_use]
pub fn explain(target: &str, local: &[NodeAddress]) -> String {
    let mut links: Vec<String> = local
        .iter()
        .filter(|a| a.prefix_len > 0)
        .map(|a| format!("{}/{} on {}", a.addr, a.prefix_len, a.iface))
        .collect();
    links.sort();
    links.dedup();
    if links.is_empty() {
        return format!(
            "this node cannot reach the rendezvous address {target}, and reports \
             no subnets of its own to compare it against"
        );
    }
    format!(
        "this node cannot reach the rendezvous address {target}. It is attached to {}. \
         The head offered an address on a link this machine is not on -- a DGX Spark \
         carries several point-to-point RoCE links, and only one of them reaches here.",
        links.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlasctl_protocol::fleet::LinkClass;

    fn addr(a: &str, prefix: u8, iface: &str) -> NodeAddress {
        NodeAddress {
            iface: iface.to_owned(),
            addr: a.to_owned(),
            class: LinkClass::Roce,
            speed_mbps: Some(200_000),
            rdma: true,
            prefix_len: prefix,
        }
    }

    /// The real topology that produced the hang. Both machines carry two RoCE
    /// ports; only one pair is on a shared link.
    fn peer_links() -> Vec<NodeAddress> {
        vec![
            addr("10.10.10.10", 30, "enp1s0f0np0"),
            addr("10.10.10.17", 30, "enp1s0f1np1"),
        ]
    }

    #[test]
    fn the_head_address_on_the_shared_link_is_reachable() {
        // 10.10.10.9/30 and 10.10.10.10/30 are the same network.
        assert!(on_local_subnet("10.10.10.9", &peer_links()));
    }

    /// The failure as it happened: the head offered its address on its *other*
    /// RoCE port, which this machine is not attached to.
    #[test]
    fn the_head_address_on_a_different_link_is_not_reachable() {
        assert!(!on_local_subnet("10.10.10.13", &peer_links()));
    }

    #[test]
    fn an_address_on_the_second_shared_link_is_reachable() {
        // 10.10.10.17/30 covers .16-.19, so .18 is its peer.
        assert!(on_local_subnet("10.10.10.18", &peer_links()));
    }

    /// A beacon reports a host address with no subnet. Treating that as /32
    /// would say "shares a link with nothing"; /24 would say the opposite.
    /// Neither is known, so it is skipped.
    #[test]
    fn an_unknown_prefix_never_claims_to_share_a_link() {
        let unknown = vec![addr("10.10.10.10", 0, "")];
        assert!(!on_local_subnet("10.10.10.9", &unknown));
        assert!(!on_local_subnet("10.10.10.10", &unknown));
    }

    #[test]
    fn a_nonsense_prefix_is_refused_rather_than_shifted() {
        // `u32 << 32` is undefined-ish territory; the guard keeps it out.
        let bad = vec![addr("10.10.10.10", 33, "x"), addr("10.10.10.10", 0, "y")];
        assert!(!on_local_subnet("10.10.10.9", &bad));
    }

    #[test]
    fn a_malformed_address_is_not_reachable() {
        assert!(!on_local_subnet("not-an-address", &peer_links()));
        assert!(!on_local_subnet("", &peer_links()));
        let bad_local = vec![addr("nonsense", 30, "x")];
        assert!(!on_local_subnet("10.10.10.9", &bad_local));
    }

    #[test]
    fn a_host_route_matches_only_itself() {
        let host = vec![addr("10.10.10.1", 32, "dummy0")];
        assert!(on_local_subnet("10.10.10.1", &host));
        assert!(!on_local_subnet("10.10.10.2", &host));
    }

    #[test]
    fn a_wider_subnet_covers_its_hosts() {
        let lan = vec![addr("192.168.68.68", 22, "wlP9s9")];
        assert!(on_local_subnet("192.168.68.73", &lan));
        assert!(!on_local_subnet("192.168.99.1", &lan));
    }

    mod explaining {
        use super::super::*;
        use super::{addr, peer_links};

        /// The message has to be enough for somebody to fix it without reading
        /// the source, so it names both the address and what this node has.
        #[test]
        fn it_names_the_address_and_this_nodes_links() {
            let msg = explain("10.10.10.13", &peer_links());
            assert!(msg.contains("10.10.10.13"), "{msg}");
            assert!(msg.contains("10.10.10.10/30 on enp1s0f0np0"), "{msg}");
            assert!(msg.contains("10.10.10.17/30 on enp1s0f1np1"), "{msg}");
        }

        #[test]
        fn a_node_with_no_known_subnets_says_so_rather_than_listing_nothing() {
            let msg = explain("10.10.10.13", &[addr("10.10.10.10", 0, "")]);
            assert!(msg.contains("no subnets of its own"), "{msg}");
        }
    }

    mod probing {
        use super::super::*;

        /// Nothing listens on the rendezvous port until rank 0 starts, so a
        /// refused connection is the expected answer from a healthy peer — and
        /// the one that separates it from a link going nowhere.
        #[test]
        fn a_refused_connection_counts_as_reachable() {
            // A port this process just released, rather than a low fixed one:
            // port 1 is not merely unbound on some hosts, it is filtered, and a
            // filtered port answers with silence rather than a refusal — so the
            // test would be asserting the opposite of what it reads.
            let port = {
                let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
                l.local_addr().expect("addr").port()
            };
            // Reported with the error the probe actually saw. "assertion failed:
            // answers(...)" names neither the port nor why, which is the whole
            // question when this fails on a platform nobody is sitting at.
            let observed = std::net::TcpStream::connect_timeout(
                &SocketAddr::new("127.0.0.1".parse().expect("ip"), port),
                PROBE_TIMEOUT,
            )
            .err()
            .map(|e| e.kind());
            assert!(
                answers("127.0.0.1", port),
                "a closed loopback port must read as reachable; \
                 connecting to {port} gave {observed:?}"
            );
        }

        /// The classification, without a socket. `answers` needs a real port
        /// to exercise, so this pins the RULE on its own: which outcomes mean
        /// "the host is there" and which mean "nothing is".
        #[test]
        fn silence_is_not_an_answer_but_a_refusal_is() {
            use std::io::ErrorKind as E;
            assert!(super::super::is_an_answer(Ok(())));
            assert!(super::super::is_an_answer(Err(E::ConnectionRefused)));
            assert!(super::super::is_an_answer(Err(E::ConnectionReset)));
            // A host that is not there answers quickly and must still read as
            // unreachable — the case a "did it come back fast?" rule would get
            // exactly backwards.
            assert!(!super::super::is_an_answer(Err(E::HostUnreachable)));
            assert!(!super::super::is_an_answer(Err(E::NetworkUnreachable)));
            assert!(!super::super::is_an_answer(Err(E::TimedOut)));
        }

        #[test]
        fn a_malformed_address_is_not_reachable() {
            assert!(!answers("not-an-address", 29500));
        }
    }
}

#[cfg(test)]
mod reachable_tests {
    use super::*;
    use atlasctl_protocol::fleet::LinkClass;

    fn a(addr: &str, prefix: u8, iface: &str, class: LinkClass, speed: u32) -> NodeAddress {
        NodeAddress {
            iface: iface.to_owned(),
            addr: addr.to_owned(),
            class,
            speed_mbps: Some(speed),
            rdma: matches!(class, LinkClass::Roce | LinkClass::InfiniBand),
            prefix_len: prefix,
        }
    }

    /// The real pair: the peer answers on two RoCE links, and this machine is
    /// attached to only one of them. Ranking by class alone is a coin flip
    /// between equals — and it picked the wrong one.
    #[test]
    fn it_picks_the_peer_link_this_machine_is_on() {
        let peer = vec![
            a("10.10.10.10", 30, "enp1s0f0np0", LinkClass::Roce, 200_000),
            a("10.10.10.17", 30, "enp1s0f1np1", LinkClass::Roce, 200_000),
        ];
        let mine = vec![
            a("10.10.10.9", 30, "enp1s0f0np0", LinkClass::Roce, 200_000),
            a("10.10.10.13", 30, "enp1s0f1np1", LinkClass::Roce, 200_000),
        ];
        assert_eq!(best_reachable(&peer, &mine).unwrap().addr, "10.10.10.10");
    }

    /// Sharing a subnet is a tiebreak between usable links, never a promotion:
    /// a shared ethernet hop must not beat a shared RoCE one.
    #[test]
    fn a_faster_shared_link_still_wins() {
        let peer = vec![
            a("192.168.1.3", 24, "eth0", LinkClass::Ethernet, 1000),
            a("10.10.10.10", 30, "enp1s0f0np0", LinkClass::Roce, 200_000),
        ];
        let mine = vec![
            a("192.168.1.2", 24, "eth0", LinkClass::Ethernet, 1000),
            a("10.10.10.9", 30, "enp1s0f0np0", LinkClass::Roce, 200_000),
        ];
        assert_eq!(best_reachable(&peer, &mine).unwrap().addr, "10.10.10.10");
    }

    /// A peer that has not reported subnets is dialled optimistically rather
    /// than declared unreachable — it may well be routable.
    #[test]
    fn an_unknown_subnet_falls_back_to_the_plain_best() {
        let peer = vec![a("10.9.9.9", 0, "", LinkClass::Roce, 200_000)];
        let mine = vec![a("10.10.10.9", 30, "enp1s0f0np0", LinkClass::Roce, 200_000)];
        assert_eq!(best_reachable(&peer, &mine).unwrap().addr, "10.9.9.9");
    }

    /// A bridge every machine happens to share must never be chosen.
    #[test]
    fn a_virtual_link_is_never_reachable_enough_to_use() {
        let peer = vec![a("172.17.0.1", 16, "docker0", LinkClass::Virtual, 0)];
        let mine = vec![a("172.17.0.1", 16, "docker0", LinkClass::Virtual, 0)];
        assert!(best_reachable(&peer, &mine).is_none());
    }

    #[test]
    fn no_addresses_yields_nothing() {
        assert!(best_reachable(&[], &[]).is_none());
    }
}
