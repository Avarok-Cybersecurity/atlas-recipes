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
    fn refusing(why: &str) -> Self {
        Self {
            answer: Err(why.to_owned()),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl crate::fleet::PeerPairing for Arc<FakePairing> {
    fn pair(
        &self,
        addr: std::net::SocketAddr,
        code: &str,
    ) -> anyhow::Result<crate::peer::pair::Paired> {
        self.calls
            .lock()
            .expect("lock")
            .push((addr, code.to_owned()));
        self.answer.clone().map_err(|e| anyhow::anyhow!(e))
    }
}

/// An agent with no way to run the ceremony must fail loudly. A pin written
/// without a key exchange would mean nothing while looking exactly like one
/// that means everything.
#[test]
fn pairing_refuses_rather_than_writing_a_pin_it_cannot_justify() {
    let t = Tmp::new("nochannel");
    let f = fleet_at(&t.0);
    let node = NodeId::from_bytes([6; 32]);
    f.observe(beacon(node, "spark-43fa", true));

    let err = f
        .pair(node, "13572468")
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
#[test]
fn a_completed_ceremony_records_the_pin_and_returns_the_words() {
    let t = Tmp::new("pairok");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::returning(node, "spark-43fa"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(node, "spark-43fa", true));

    let out = f.pair(node, "13572468").expect("pairs");
    assert_eq!(out.node, node);
    assert_eq!(out.verification, "abcd-ef01");
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
#[test]
fn a_machine_that_is_not_the_one_selected_is_refused() {
    let t = Tmp::new("wrongid");
    let selected = NodeId::from_bytes([6; 32]);
    let impostor = NodeId::from_bytes([9; 32]);
    let driver = Arc::new(FakePairing::returning(impostor, "not-who-you-picked"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(selected, "spark-43fa", true));

    let err = f.pair(selected, "13572468").expect_err("must refuse");
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
#[test]
fn a_failed_ceremony_writes_no_pin() {
    let t = Tmp::new("badcode");
    let node = NodeId::from_bytes([6; 32]);
    let driver = Arc::new(FakePairing::refusing("key confirmation failed"));
    let f = fleet_at(&t.0).with_pairing(Box::new(Arc::clone(&driver)));
    f.observe(beacon(node, "spark-43fa", true));

    let err = f.pair(node, "13572468").expect_err("must refuse");
    assert!(err.to_string().contains("key confirmation"), "{err}");
    assert!(!PinStore::new(&t.0).is_pinned(node).expect("reads"));
}
