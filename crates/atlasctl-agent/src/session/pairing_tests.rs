// SPDX-License-Identifier: AGPL-3.0-only

//! Two-phase pairing at the session boundary.
//!
//! Split from `session/tests.rs` on the 500-line cap. The seam is real: these
//! are about when trust is written, and the rest of that file is about launches
//! and the handshake.

use super::tests::{Fixture, TOKEN};
use atlasctl_protocol::{ClientMsg, ServerMsg};

// ---- two-phase pairing ---------------------------------------------------
//
// The states worth pinning are the refusals. A confirm that works is the easy
// half; a confirm that must NOT work is where an unverified pin would sneak in.

/// Confirming with nothing pending must refuse rather than guess.
///
/// The words belong to one exchange. A confirm arriving without one — a
/// reconnected tab, a replayed frame, a second click after the socket dropped —
/// has no exchange to be about, and treating it as consent would trust a
/// machine on the strength of a message that names no key.
#[test]
fn confirming_with_no_exchange_in_flight_trusts_nothing() {
    let f = Fixture::new();
    let mut s = f.ready();
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([3u8; 32]);
    let out = s.handle(ClientMsg::ConfirmPairing { id: 7, node });
    match &out[0] {
        ServerMsg::PairDecision {
            trusted, detail, ..
        } => {
            assert!(!trusted, "nothing may be trusted: {out:?}");
            assert!(
                detail.contains("no exchange"),
                "must say why rather than failing silently: {detail}"
            );
        }
        other => panic!("expected a decision, got {other:?}"),
    }
}

/// Rejecting with nothing pending is also honest about it, and still trusts
/// nothing.
#[test]
fn rejecting_with_no_exchange_in_flight_is_not_an_error() {
    let f = Fixture::new();
    let mut s = f.ready();
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([4u8; 32]);
    match &s.handle(ClientMsg::RejectPairing { id: 8, node })[0] {
        ServerMsg::PairDecision { trusted, .. } => assert!(!trusted),
        other => panic!("expected a decision, got {other:?}"),
    }
}

/// Unpair answers a trust decision, not a pairing result.
///
/// It runs no exchange, so a `PairResult` would have to claim `exchanged:
/// false` — true only in the sense that nothing happened, which is not what
/// that field is for.
#[test]
fn unpair_answers_a_decision_rather_than_an_exchange() {
    let f = Fixture::new();
    let mut s = f.ready();
    let node = atlasctl_protocol::fleet::NodeId::from_bytes([5u8; 32]);
    // No fleet on this fixture, so it reports NotReady — the point here is only
    // that nothing in the pairing path answers with the wrong shape.
    let out = s.handle(ClientMsg::UnpairPeer { id: 9, node });
    assert!(
        !matches!(out[0], ServerMsg::PairResult { .. }),
        "unpair must not answer with an exchange-shaped reply: {out:?}"
    );
}

/// A version 1 client must be refused outright.
///
/// It would read `exchanged` as the old `paired` and show a machine as trusted
/// that this agent has not accepted. Refusing at the handshake is the point of
/// the bump, not an inconvenience.
#[test]
fn a_protocol_1_client_is_refused_because_it_would_misread_exchanged() {
    let f = Fixture::new();
    let mut s = f.session();
    let out = s.handle(ClientMsg::Hello {
        protocol_version: 1,
        token: TOKEN.into(),
    });
    assert!(
        !matches!(out[0], ServerMsg::Ready { .. }),
        "an older client must not complete the handshake: {out:?}"
    );
}
