// SPDX-License-Identifier: AGPL-3.0-only

//! The checks `doctor` runs beyond docker and sparkrun.
//!
//! Split from [`super::doctor`] on the 500-line cap, and because these share a
//! theme the other two do not: every one of them is a failure somebody actually
//! hit during onboarding, diagnosed after the fact from an error that named the
//! wrong thing. A check here exists to move that diagnosis to before the
//! attempt.

use std::fmt::Write as _;

/// One finding, so the verdict is decided by a pure function a test can drive.
///
/// The alternative — printing inside each check and counting as we go — is what
/// the first two checks do, and it means the wording can only be verified by
/// running the command and reading it. These are the checks whose wording has
/// already been wrong once.
#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    /// `ok`, or the problem in the operator's terms.
    pub line: String,
    /// Whether this counts toward the problem tally.
    pub problem: bool,
}

impl Finding {
    fn ok(what: &str, detail: &str) -> Self {
        Self {
            line: format!("{:<9} ok ({detail})", format!("{what}:")),
            problem: false,
        }
    }

    fn bad(what: &str, detail: &str, remedy: &[&str]) -> Self {
        let mut line = format!("{:<9} PROBLEM — {detail}", format!("{what}:"));
        for r in remedy {
            let _ = write!(line, "\n\x20         {r}");
        }
        Self {
            line,
            problem: true,
        }
    }
}

/// Can this machine keep its identity and pins?
///
/// The failure this catches: the config directory exists but is not writable by
/// the running user, so `agent run` dies on `open(O_CREAT)` of `browser.token`
/// with an EACCES that names a file rather than the directory, and the operator
/// goes looking for the file.
///
/// The DIAGNOSIS is not re-derived here. `usable_config_dir` already tells
/// "cannot create" apart from "not writable by you" and carries the right
/// remedy for each; doctor surfaces that verdict verbatim rather than forming a
/// second opinion that could disagree with the one `agent run` prints.
#[must_use]
pub fn config_dir(state: ConfigDirState) -> Finding {
    match state {
        ConfigDirState::Writable(path) => Finding::ok("config", &path),
        ConfigDirState::Unusable(why) => Finding::bad("config", &why, &[]),
    }
}

/// What `config_dir` was told about the directory.
#[derive(Debug)]
pub enum ConfigDirState {
    /// Usable, at this path.
    Writable(String),
    /// Not usable, with `usable_config_dir`'s own explanation and remedy.
    Unusable(String),
}

/// Is an agent answering, and is that what the operator expects?
///
/// Not running is NOT a problem on a machine nobody has set up yet, so this
/// reports rather than accuses — an operator running `doctor` before installing
/// anything should not be told they have a fault.
#[must_use]
pub fn agent(listening: bool, port: u16) -> Finding {
    if listening {
        Finding::ok("agent", &format!("listening on 127.0.0.1:{port}"))
    } else {
        Finding {
            line: format!("{:<9} not running", "agent:"),
            problem: false,
        }
    }
}

/// Could another machine dial this one?
///
/// The failure this catches: a laptop on Wi-Fi advertised no address at all,
/// because the address list was filtered for links that can carry a COLLECTIVE
/// rather than links that can be reached. The operator saw an empty command
/// where the invitation should have been, with nothing explaining it.
///
/// Zero dialable addresses is a real problem for a machine expected to join a
/// fleet, and it is invisible until an invitation comes out blank.
#[must_use]
pub fn reachable(addresses: &[(String, String)]) -> Finding {
    if let Some((iface, addr)) = addresses.first() {
        let more = addresses.len().saturating_sub(1);
        let detail = if more == 0 {
            format!("{addr} on {iface}")
        } else {
            format!("{addr} on {iface}, and {more} more")
        };
        Finding::ok("network", &detail)
    } else {
        Finding::bad(
            "network",
            "no address another machine could dial",
            &[
                "Only loopback or virtual interfaces are up.",
                "Connect this machine to the network you want the fleet on.",
                "Until then it cannot be discovered and its invitations have nowhere to point.",
            ],
        )
    }
}

/// The interfaces could not be READ, which is not the same as there being none.
///
/// The distinction matters because the remedies are unrelated: "connect this
/// machine to a network" is useless advice to someone whose `/sys/class/net`
/// could not be listed. Collapsing an error into an empty list is the same
/// mistake as rendering an absent metric as zero — it turns "we do not know"
/// into a confident claim.
#[must_use]
pub fn unreadable_interfaces(why: &str) -> Finding {
    Finding::bad(
        "network",
        &format!("this machine's network interfaces could not be read: {why}"),
        &["Whether another machine could reach this one is unknown, not no."],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usable_config_dir_is_not_a_problem_and_names_the_path() {
        let f = config_dir(ConfigDirState::Writable("/home/x/.config/atlasctl".into()));
        assert!(!f.problem);
        assert!(f.line.contains("/home/x/.config/atlasctl"), "{}", f.line);
    }

    #[test]
    fn an_unusable_config_dir_carries_its_own_explanation_through() {
        // Verbatim, because `usable_config_dir` already chose the wording and
        // the remedy. Doctor paraphrasing it is how two messages about one
        // fault start disagreeing.
        let f = config_dir(ConfigDirState::Unusable(
            "owned by root; run --config-dir".into(),
        ));
        assert!(f.problem);
        assert!(f.line.contains("owned by root"), "{}", f.line);
        assert!(f.line.contains("--config-dir"), "{}", f.line);
    }

    /// An operator running `doctor` before installing anything has no fault.
    #[test]
    fn an_agent_that_is_not_running_is_reported_but_not_counted() {
        let f = agent(false, 34333);
        assert!(!f.problem, "not-installed-yet is not a problem: {}", f.line);
        assert!(f.line.contains("not running"), "{}", f.line);
    }

    #[test]
    fn a_listening_agent_names_the_port_it_answered_on() {
        let f = agent(true, 34333);
        assert!(!f.problem);
        assert!(f.line.contains("34333"), "{}", f.line);
    }

    /// The bug this check exists for: an invitation with nowhere to point.
    #[test]
    fn no_dialable_address_is_a_problem_and_says_what_it_costs() {
        let f = reachable(&[]);
        assert!(f.problem);
        assert!(f.line.contains("no address"), "{}", f.line);
        assert!(
            f.line.contains("invitations"),
            "the operator needs to know what it breaks, not just that it is: {}",
            f.line
        );
    }

    /// "Could not look" and "looked and found none" are different facts with
    /// unrelated remedies. Reporting the first as the second is the same
    /// mistake as rendering an absent metric as zero.
    #[test]
    fn an_unreadable_interface_list_is_not_reported_as_having_no_addresses() {
        let f = unreadable_interfaces("permission denied reading /sys/class/net");
        assert!(f.problem);
        assert!(f.line.contains("could not be read"), "{}", f.line);
        assert!(
            f.line.contains("permission denied"),
            "the cause is the useful part: {}",
            f.line
        );
        // The remedy for an empty list must not appear here.
        assert!(
            !f.line.contains("Connect this machine"),
            "plugging in a cable does not fix an unreadable /sys: {}",
            f.line
        );
        assert!(f.line.contains("unknown, not no"), "{}", f.line);
    }

    #[test]
    fn a_dialable_address_names_one_and_counts_the_rest() {
        let f = reachable(&[
            ("enp1s0f0np0".into(), "10.10.10.9".into()),
            ("wlp3s0".into(), "192.168.1.24".into()),
        ]);
        assert!(!f.problem);
        assert!(f.line.contains("10.10.10.9"), "{}", f.line);
        assert!(f.line.contains("1 more"), "{}", f.line);
    }
}
