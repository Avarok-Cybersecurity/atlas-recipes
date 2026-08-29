// SPDX-License-Identifier: AGPL-3.0-only

//! Pairing a discovered machine, driven by the browser channel.
//!
//! Split from [`super::tests`] for size. These cover what `LocalFleet::pair`
//! does once a ceremony can actually run: which address it dials, that the
//! machine which answered is the one that was selected, and that nothing
//! short of a completed exchange leaves trust behind.

use super::tests::{Tmp, beacon, fleet_at};
use crate::fleet::FleetView as _;
use crate::identity::PinStore;
use atlasctl_protocol::fleet::NodeId;
use std::sync::Arc;

/// A recording pairing driver, so the ceremony's callers are testable without
/// a second machine.
struct FakePairing {
    /// What the ceremony will answer with, or the refusal it will produce.
    answer: Result<crate::peer::pair::Paired, String>,
    calls: std::sync::Mutex<Vec<(std::net::SocketAddr, String)>>,
}

impl FakePairing {
    fn returning(node: NodeId, name: &str) -> Self {
        Self {
            answer: Ok(crate::peer::pair::Paired {
                node,
                public_key: hex::encode([7u8; 32]),
                name: name.to_owned(),
                verification: "abcd-ef01".to_owned(),
            }),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
    /// A machine that never answered — the wrong network, a firewall, a box
    /// asleep. Distinct from [`Self::refusing`] because only this one lets the
    /// walk move on without spending an attempt.
    fn unreachable() -> Self {
        Self {
            answer: Err(String::new()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
    fn refusing(why: &str) -> Self {
        Self {
            answer: Err(why.to_owned()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl crate::fleet::PeerPairing for Arc<FakePairing> {
    fn pair<'a>(
        &'a self,
        addr: std::net::SocketAddr,
        code: &'a str,
    ) -> crate::BoxFut<'a, anyhow::Result<crate::peer::pair::Paired>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("lock")
                .push((addr, code.to_owned()));
            match self.answer.clone() {
                Ok(p) => Ok(p),
                // An empty reason stands for a transport failure, which is carried
                // as a real `io::Error` so the production predicate sees what it
                // would see in the field rather than a string we shaped for it.
                Err(e) if e.is_empty() => Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "connection refused",
                ))
                .context("dialling the peer")),
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        })
    }
}

/// An agent with no way to run the ceremony must fail loudly. A pin written
/// without a key exchange would mean nothing while looking exactly like one
/// that means everything.
#[tokio::test]
async fn pairing_refuses_rather_than_writing_a_pin_it_cannot_justify() {
    let t = Tmp::new("nochannel");
    let f = fleet_at(&t.0);
    let node = NodeId::from_bytes([6; 32]);
    f.observe(beacon(node, "spark-43fa", true));

    let err = f
        .pair(node, "13572468")
        .await
        .expect_err("must not fake a pairing");
    assert!(
        err.to_string().contains("cannot run a pairing ceremony"),
        "{err}"
    );
    assert!(
        !PinStore::new(&t.0).is_pinned(node).expect("reads"),
        "a failed pairing must leave no trust behind"
    );
}

/// The whole point: a browser can now pair a discovered machine. This is what
/// `fleet/listing.rs` used to refuse outright, which is why the pair dialog
/// existed but could never succeed.
#[tokio::test]
async fn a_completed_ceremony_records_the_pin_and_returns_the_words() {
    let t = Tmp::new("pairok");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::returning(node, "spark-43fa"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(node, "spark-43fa", true));

    let out = f.pair(node, "13572468").await.expect("pairs");
    assert_eq!(out.node, node);
    assert_eq!(out.verification, "abcd-ef01");
    // The exchange completed and NOTHING is trusted yet. This is the whole
    // point of two-phase pairing: the words exist to be compared, and a
    // comparison that happens after the pin is written is a formality.
    assert!(
        !PinStore::new(&t.0).is_pinned(node).expect("reads"),
        "pair() must not write a pin; trust() does"
    );

    // And accepting it writes one.
    f.trust(&out, false).expect("trusts");
    assert!(PinStore::new(&t.0).is_pinned(node).expect("reads"));

    // Dialled the address the beacon advertised, on the peer port, with the
    // operator's code — not the browser port, and not a code of its own.
    let calls = driver.calls.lock().expect("lock");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.port(), 34334);
    assert_eq!(calls[0].1, "13572468");
}

/// The ceremony authenticates *a* peer; this checks it is the peer the
/// operator selected. Without it, whatever answers on that address gets pinned
/// under the identity of the machine that was chosen — which is the one thing
/// an address from an unauthenticated beacon must not be able to buy.
#[tokio::test]
async fn a_machine_that_is_not_the_one_selected_is_refused() {
    let t = Tmp::new("wrongid");
    let selected = NodeId::from_bytes([6; 32]);
    let impostor = NodeId::from_bytes([9; 32]);
    let driver = Arc::new(FakePairing::returning(impostor, "not-who-you-picked"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(selected, "spark-43fa", true));

    let err = f.pair(selected, "13572468").await.expect_err("must refuse");
    assert!(err.to_string().contains("was selected"), "{err}");
    let pins = PinStore::new(&t.0);
    assert!(!pins.is_pinned(selected).expect("reads"));
    assert!(
        !pins.is_pinned(impostor).expect("reads"),
        "and certainly must not pin whoever actually answered"
    );
}

/// A wrong code and a relayed connection both surface here as a failed
/// ceremony, and neither may leave trust behind.
#[tokio::test]
async fn a_failed_ceremony_writes_no_pin() {
    let t = Tmp::new("badcode");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::refusing("key confirmation failed"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(node, "spark-43fa", true));

    let err = f.pair(node, "13572468").await.expect_err("must refuse");
    assert!(err.to_string().contains("key confirmation"), "{err}");
    assert!(!PinStore::new(&t.0).is_pinned(node).expect("reads"));
}

/// Refusing the words must leave nothing behind.
///
/// Before this was two-phase the pin already existed by now, so a refusal had
/// to be a REMOVAL — and a removal that failed left a machine trusted that the
/// operator had explicitly rejected. There is nothing to remove now.
#[tokio::test]
async fn an_exchange_that_is_never_trusted_writes_nothing() {
    let t = Tmp::new("never-trusted");
    let node = NodeId::from_bytes([9u8; 32]);
    let driver = Arc::new(FakePairing::returning(node, "spark-28c2"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(node, "spark-28c2", true));

    let out = f.pair(node, "13572468").await.expect("pairs");
    drop(out); // the operator said the words did not match

    assert!(
        !PinStore::new(&t.0).is_pinned(node).expect("reads"),
        "a refused exchange must never have touched the pin store"
    );
}

/// A beacon from a machine that offers several links, like a DGX with two
/// RoCE addresses and one on the ordinary LAN.
fn beacon_at(id: NodeId, name: &str, addrs: &[&str]) -> crate::discovery::Beacon {
    let mut b = beacon(id, name, true);
    b.addresses = addrs
        .iter()
        .map(|a| a.parse::<std::net::IpAddr>().expect("addr"))
        .collect();
    b
}

/// The code allows three attempts and a DGX advertises three addresses, so a
/// walk that treats a REFUSAL as a reason to try the next one turns a single
/// mistyped code into a lockout — before the operator has had one real go.
#[tokio::test]
async fn a_machine_that_answered_and_refused_costs_exactly_one_attempt() {
    let t = Tmp::new("one-attempt");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::refusing("key confirmation failed"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon_at(
        node,
        "dgx1",
        &["10.10.10.9", "10.10.10.13", "192.168.68.68"],
    ));

    f.pair(node, "13572468").await.expect_err("must refuse");
    assert_eq!(
        driver.calls.lock().expect("lock").len(),
        1,
        "every address here is the SAME machine; a refusal already spent an \
         attempt, so the walk must stop rather than spend the rest"
    );
}

/// The other half: when nothing answers, the next address is free, and it is
/// the whole reason more than one is offered — a laptop can reach only the
/// last of a DGX's three.
#[tokio::test]
async fn an_address_that_never_answers_moves_on_to_the_next() {
    let t = Tmp::new("walk-on");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::unreachable());
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon_at(
        node,
        "dgx1",
        &["10.10.10.9", "10.10.10.13", "192.168.68.68"],
    ));

    f.pair(node, "13572468")
        .await
        .expect_err("nothing answered anywhere");
    let calls = driver.calls.lock().expect("lock");
    assert_eq!(calls.len(), 3, "all three links must be tried");
    assert_eq!(
        calls[2].0.ip().to_string(),
        "192.168.68.68",
        "the LAN address is last, and is the only one a laptop could use"
    );
}
