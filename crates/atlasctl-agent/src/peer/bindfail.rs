// SPDX-License-Identifier: AGPL-3.0-only

//! Why the peer channel could not start.
//!
//! The old line said what had happened and what it cost — "could not bind
//! 34334: Address already in use … other machines will not be able to reach
//! this one" — and stopped one sentence short of the only thing the operator
//! can act on: WHO holds the port.
//!
//! In practice there is one answer. The peer port is not configurable on
//! `agent run`, so the holder is almost always this machine's other agent —
//! a service installed by `install.sh` plus a second copy started by hand,
//! which is a thing that happens the first time anyone experiments. The
//! browser port differs (it is a flag), so the second copy starts, serves a
//! page, and is silently unreachable by every other machine.
//!
//! Pure, so the wording is tested rather than discovered in the field.

/// The operator-facing reason the peer channel is off.
#[must_use]
pub fn peer_bind_failure(port: u16, kind: std::io::ErrorKind, detail: &str) -> String {
    let consequence = "other machines will not be able to reach this one, and this \
                       one cannot be added to a fleet";
    match kind {
        std::io::ErrorKind::AddrInUse => format!(
            "peer channel disabled: {port} is already in use — {consequence}.\n\
             \n\
             The peer port is the same for every agent, so this is almost always \
             another `atlasctl agent` already running here; `atlasctl agent status` \
             will say. Stop that one, or leave it running and use it — two agents \
             on one machine only ever gives you the one that got the port."
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "peer channel disabled: not allowed to bind {port} ({detail}) — \
             {consequence}.\n\
             \n\
             A sandbox or a local policy is refusing the port, not another \
             process: nothing here needs privileges, so this is the environment \
             rather than the fleet."
        ),
        _ => format!("peer channel disabled: could not bind {port}: {detail} — {consequence}"),
    }
}

#[cfg(test)]
mod tests {
    use super::peer_bind_failure;
    use std::io::ErrorKind;

    /// The case that actually happens, and the one the old message could not
    /// help with: a second agent on a machine that already has one.
    #[test]
    fn a_taken_port_names_the_other_agent_and_how_to_check() {
        let m = peer_bind_failure(34334, ErrorKind::AddrInUse, "Address already in use");
        assert!(
            m.contains("agent status"),
            "must say how to confirm it: {m}"
        );
        assert!(
            m.contains("another `atlasctl agent`"),
            "must name the likely holder: {m}"
        );
        assert!(
            m.contains("cannot be added to a fleet"),
            "and must keep the consequence: {m}"
        );
    }

    /// A refused port is not a busy port, and sending someone to hunt for a
    /// second agent that does not exist wastes the time the message is for.
    #[test]
    fn a_refused_port_does_not_blame_another_agent() {
        let m = peer_bind_failure(34334, ErrorKind::PermissionDenied, "denied");
        assert!(!m.contains("another `atlasctl agent`"), "{m}");
        assert!(m.contains("sandbox") || m.contains("policy"), "{m}");
    }

    /// Anything else still reports verbatim rather than guessing: an invented
    /// cause is worse than an unexplained one.
    #[test]
    fn an_unrecognised_failure_is_passed_through_with_its_own_words() {
        let m = peer_bind_failure(
            34334,
            ErrorKind::AddrNotAvailable,
            "cannot assign requested address",
        );
        assert!(m.contains("cannot assign requested address"), "{m}");
        assert!(!m.contains("agent status"), "no invented remedy: {m}");
    }
}
