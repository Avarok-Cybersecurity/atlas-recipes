// SPDX-License-Identifier: AGPL-3.0-only

//! Writing to the pin store, and the empty vitals every caller needs.
//!
//! Split from `fleet.rs` on the 500-line cap. The seam is real: `LocalFleet` is
//! the live view of who is out there, and these are the free functions that
//! WRITE what this machine has decided to remember. They take a `&PinStore`
//! rather than `&self` precisely because they are reachable from the CLI paths
//! that have no fleet at all.

use anyhow::Result;
use atlasctl_protocol::fleet::{DisplayName, NodeId, NodeVitals};

use crate::identity::{Pin, PinStore};

/// Record a completed pairing.
///
/// Separate from [`FleetView::pair`] so the ceremony's transport and its
/// bookkeeping are not tangled: whatever drives the exchange calls this once,
/// and only once key confirmation has passed.
///
/// # Errors
/// If the pin store cannot be written.
pub fn record_pairing(
    pins: &PinStore,
    node: NodeId,
    public_key_hex: &str,
    name: DisplayName,
    now_unix: u64,
    last_address: Option<String>,
    controller: bool,
) -> Result<()> {
    pins.add(Pin {
        id: node,
        public_key: public_key_hex.to_owned(),
        name,
        paired_at: now_unix,
        last_address,
        // Pairing authenticates; it does not authorize control. `controller`
        // is true only when a human said so in the same breath as trusting
        // the machine (`allow_control` on the confirm or the join window) —
        // never as a side effect of the ceremony itself, and otherwise the
        // grant stays a separate act (`atlasctl peer grant-control`).
        controller,
    })
}

/// Remember where a paired peer was last seen.
///
/// Called when a beacon refreshes, so the address survives an agent restart.
///
/// # Errors
/// If the pin store cannot be read or written.
pub fn remember_address(pins: &PinStore, node: NodeId, addr: &str) -> Result<()> {
    let mut all = pins.load()?;
    if let Some(pin) = all.get_mut(&node)
        && pin.last_address.as_deref() != Some(addr)
    {
        pin.last_address = Some(addr.to_owned());
        let updated = pin.clone();
        pins.add(updated)?;
    }
    Ok(())
}

/// Vitals a provider could not supply at all.
#[must_use]
pub fn no_vitals() -> NodeVitals {
    NodeVitals::default()
}
