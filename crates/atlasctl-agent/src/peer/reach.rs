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
}
