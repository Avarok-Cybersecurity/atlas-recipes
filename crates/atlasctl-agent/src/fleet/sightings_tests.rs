// SPDX-License-Identifier: AGPL-3.0-only

//! What the sightings table will and will not hold.
//!
//! Split from `fleet/tests.rs` on the 500-line cap. The seam is real: that file
//! is about what the fleet view REPORTS, and these are about what an
//! unauthenticated network can make it STORE.

use super::tests::{Tmp, beacon, fleet_at};
use crate::fleet::record_pairing;
use crate::identity::{Identity, PinStore};
use atlasctl_protocol::fleet::DisplayName;
use atlasctl_protocol::fleet::NodeId;

/// mDNS is unauthenticated: anything on the LAN can announce any id. Without a
/// bound, a burst of invented ids grows the sightings table for the whole
/// `UNREACHABLE_AFTER` window at whatever rate the network allows.
#[test]
fn a_flood_of_invented_beacons_cannot_grow_the_table_without_bound() {
    let t = Tmp::new("flood");
    let f = fleet_at(&t.0);
    // Fixed numbers, NOT `MAX_SIGHTINGS`: a test that derives both its input and
    // its expectation from the constant passes for any value of it, including
    // one so large the bound is decorative. These say what a fleet console must
    // never do, independently of what the constant happens to be today.
    const FLOOD: usize = 5_000;
    const SANE_CEILING: usize = 1_000;
    for i in 0..FLOOD {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&(i as u64).to_le_bytes());
        f.observe(beacon(NodeId::from_bytes(id), "stranger", false));
    }
    let n = f.lock_seen().expect("lock").len();
    assert!(
        n <= SANE_CEILING,
        "{FLOOD} invented beacons left {n} sightings; the design target is 1-8 machines"
    );
}

/// The bound must never cost you a machine you actually paired.
#[test]
fn a_paired_machine_is_still_recorded_once_the_table_is_full() {
    let t = Tmp::new("flood-pinned");
    let f = fleet_at(&t.0);
    let mine = NodeId::from_bytes([9; 32]);
    crate::fleet::record_pairing(
        &f.pins,
        mine,
        "ab".repeat(32).as_str(),
        atlasctl_protocol::fleet::DisplayName::new("real-peer"),
        0,
        None,
        false,
    )
    .expect("pin");

    for i in 0..5_000 {
        let mut id = [0u8; 32];
        id[..8].copy_from_slice(&(i as u64).to_le_bytes());
        f.observe(beacon(NodeId::from_bytes(id), "stranger", false));
    }
    // The real peer announces itself after the flood has filled the table.
    f.observe(beacon(mine, "real-peer", true));
    assert!(
        f.lock_seen().expect("lock").contains_key(&mine),
        "a pinned peer must never be crowded out by strangers"
    );
}

/// A pin store that cannot be READ is not a fleet with no pins.
///
/// `prune` keeps pinned peers regardless of age — that is its stated contract,
/// because a machine you paired is yours whether or not it is switched on. Read
/// the pins as "none" on an I/O error and the contract inverts: the idle
/// pinned peers are exactly the ones dropped, and they stay dropped for as long
/// as the file is unreadable.
#[test]
fn an_unreadable_pin_store_does_not_evict_the_fleet_it_cannot_see() {
    let t = Tmp::new("prune-blind");
    let f = fleet_at(&t.0);
    let mine = NodeId::from_bytes([7; 32]);
    crate::fleet::record_pairing(
        &f.pins,
        mine,
        "cd".repeat(32).as_str(),
        atlasctl_protocol::fleet::DisplayName::new("switched-off-dgx"),
        0,
        None,
        false,
    )
    .expect("pin");
    f.observe(beacon(mine, "switched-off-dgx", true));

    // It has since been switched off for longer than the window: without the
    // pin, this sighting is stale and prune drops it.
    {
        let mut seen = f.lock_seen().expect("lock");
        let s = seen.get_mut(&mine).expect("just observed");
        s.at = std::time::Instant::now()
            .checked_sub(crate::fleet::UNREACHABLE_AFTER + std::time::Duration::from_secs(1))
            .expect("the test clock is past the window");
    }

    // Now the pin file becomes unreadable — a partial write, a bad disk, a
    // half-finished save.
    std::fs::write(t.0.join("peers.json"), b"{ this is not json").expect("corrupt");

    f.prune();
    assert!(
        f.lock_seen().expect("lock").contains_key(&mine),
        "a pinned peer was evicted because its pin file could not be parsed; \
         the operator would see their fleet vanish over an I/O error"
    );
}

/// An mDNS beacon is unauthenticated and a `NodeId` is a public key
/// fingerprint that rides in every one of them, so "claims to be a peer you
/// trust" is not a thing an attacker has to guess.
///
/// `observe` used to call `remember_address`, which writes `last_address` into
/// the PIN store — three lines under a comment promising it "only ever writes
/// into the sightings table". Anything on the LAN could therefore rewrite where
/// a trusted machine was believed to live, persistently: the agent dials the
/// attacker, the real machine reads as unreachable, and a restart does not
/// clear it. SPKI pinning refuses the impersonation, so this was redirection
/// rather than key compromise — which is exactly the kind of bug that survives,
/// because nothing visibly breaks.
#[test]
fn a_beacon_cannot_move_a_pinned_peer() {
    let t = Tmp::new("beacon-move");
    let peer = Identity::generate();
    let pins = PinStore::new(&t.0);
    record_pairing(
        &pins,
        peer.id(),
        &hex::encode(peer.public().as_bytes()),
        DisplayName::new("spark-43fa"),
        0,
        Some("10.10.10.10".to_owned()),
        false,
    )
    .expect("pin");

    // A hostile beacon claiming that peer's id, from somewhere else entirely.
    let mut hostile = beacon(peer.id(), "spark-43fa", true);
    hostile.addresses = vec!["10.10.10.99".parse().expect("addr")];
    fleet_at(&t.0).observe(hostile);

    assert_eq!(
        pins.load().expect("read")[&peer.id()]
            .last_address
            .as_deref(),
        Some("10.10.10.10"),
        "an unauthenticated beacon must not rewrite a trusted peer's address"
    );
}

/// A beacon has always carried the peer's own `peer_port`, and the poll threw
/// it away — every peer was dialled on THIS agent's port instead. That is
/// correct only while every agent binds the same one, which is true today and
/// is exactly the assumption that makes a per-machine port impossible to add.
///
/// It is also why a pin can only remember an IP: storing a port would persist
/// a number nothing consults.
#[test]
fn a_peer_is_dialled_on_the_port_it_advertised() {
    let t = Tmp::new("dial-port");
    let f = fleet_at(&t.0);
    let mine = NodeId::from_bytes([3; 32]);
    record_pairing(
        &f.pins,
        mine,
        "ef".repeat(32).as_str(),
        DisplayName::new("odd-port-peer"),
        0,
        Some("10.10.10.10".to_owned()),
        false,
    )
    .expect("pin");

    let mut b = beacon(mine, "odd-port-peer", true);
    b.peer_port = 34999;
    f.observe(b);

    let dials = f.dialable_peers().expect("reads");
    let (_, dial) = dials
        .iter()
        .find(|(id, _)| *id == mine)
        .expect("a pinned, announcing peer is dialable");
    assert_eq!(
        dial.port,
        Some(34999),
        "the port the PEER announced must reach the dial, not this agent's own"
    );
}

/// A peer that is not announcing has no port to offer, and the caller falls
/// back to the default rather than inventing one — `None` is "it did not say",
/// which is the same distinction the vitals carry.
#[test]
fn a_silent_peer_offers_no_port_rather_than_a_guess() {
    let t = Tmp::new("dial-port-silent");
    let f = fleet_at(&t.0);
    let mine = NodeId::from_bytes([4; 32]);
    record_pairing(
        &f.pins,
        mine,
        "ab".repeat(32).as_str(),
        DisplayName::new("switched-off"),
        0,
        Some("10.10.10.11".to_owned()),
        false,
    )
    .expect("pin");

    let dials = f.dialable_peers().expect("reads");
    let (_, dial) = dials
        .iter()
        .find(|(id, _)| *id == mine)
        .expect("still listed");
    assert_eq!(dial.addr, "10.10.10.11", "the pin's address stands");
    assert_eq!(dial.port, None, "no sighting means no advertised port");
}
