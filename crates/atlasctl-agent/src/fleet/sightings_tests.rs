// SPDX-License-Identifier: AGPL-3.0-only

//! What the sightings table will and will not hold.
//!
//! Split from `fleet/tests.rs` on the 500-line cap. The seam is real: that file
//! is about what the fleet view REPORTS, and these are about what an
//! unauthenticated network can make it STORE.

use super::tests::{Tmp, beacon, fleet_at};
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
