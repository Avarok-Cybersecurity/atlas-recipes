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

/// Is there room to pull an image and a model?
///
/// A launch pulls the runtime image and, when the weights are not already
/// cached, the model itself. Measured on a GB10 box: `avarok/atlas-gb10` is
/// 2.85 GB and the smallest model in the shipped catalogue is 21.8 GB, so
/// below their sum a launch cannot succeed. What the operator sees instead is
/// a docker pull error, or a download that stops partway and leaves a cache
/// entry that looks present — neither of which mentions the disk.
///
/// An absolute floor, deliberately not a percentage. 5% of a 3.7 TB array is
/// 185 GB, which would call a box with 172 GB free — comfortably enough for
/// any launch here — a problem.
pub fn disk_space(free_bytes: u64, path: &str) -> Finding {
    let Some(why) = disk_caution(free_bytes, path) else {
        let gb = free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        return Finding::ok("disk", &format!("{gb:.0} GB free on {path}"));
    };
    Finding::bad(
        "disk",
        &why,
        &[
            "atlasctl status            # stop anything you are not using",
            "docker image prune -a      # reclaim images no container needs",
            "du -sh ~/.cache/huggingface/hub/models--*   # then delete a model you can re-pull",
        ],
    )
}

/// The same judgement, as a one-line caution rather than a `doctor` finding.
///
/// `doctor` is the command an operator runs when something is already wrong.
/// `run` is where the cost lands: a launch started on a full disk pulls for
/// forty minutes and then fails with a docker error that never mentions space.
/// Sharing the floor rather than copying it is what keeps the two from drifting
/// into disagreeing about what "enough" means.
///
/// `None` when there is room.
#[must_use]
pub fn disk_caution(free_bytes: u64, path: &str) -> Option<String> {
    /// Below the runtime image plus the smallest shipped model, a launch that
    /// has to pull cannot succeed. An absolute floor, deliberately not a
    /// percentage: 5% of a 3.7 TB array is 185 GB, which would call a box with
    /// 172 GB free — comfortably enough for any launch here — a problem.
    const FLOOR: u64 = 25 * 1024 * 1024 * 1024;
    if free_bytes >= FLOOR {
        return None;
    }
    let gb = free_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    Some(format!(
        "only {gb:.1} GB free on {path}. The runtime image is ~3 GB and the \
         smallest model in the catalogue is ~22 GB, so a launch that has to \
         pull either will fail — usually with a message about docker or a \
         truncated download, not about space."
    ))
}

/// When `df` could not be read at all.
///
/// Reported as ok, not as a problem. An unreadable `df` says nothing about
/// whether there is room, and `doctor` exits non-zero on a problem — so
/// guessing here would fail a perfectly healthy machine.
#[must_use]
pub fn disk_unknown() -> Finding {
    Finding::ok("disk", "not measurable here")
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
        // Names the port it asked about, and says the flag.
        //
        // `agent install --port` is first-class (cli.rs:113), and this check
        // only ever probes the default. An operator who installed on another
        // port -- or with `--no-browser`, which binds none -- read a flat
        // "not running" about an agent that was running fine. `agent status`
        // was fixed for exactly this (`agentinfo::status_advice`, whose comment
        // records that the old advice sent people to start a SECOND agent that
        // then failed to bind); doctor was not.
        Finding {
            line: format!("{:<9} not running on 127.0.0.1:{port}", "agent:"),
            problem: false,
        }
    }
}

/// Is the listener other machines actually connect to open?
///
/// Distinct from [`agent`], which probes the BROWSER port. They are separate
/// listeners with separate failure modes, and only one of them was ever
/// checked here — so doctor could report a healthy agent and a dialable
/// address for a machine that could not accept a single peer.
///
/// That combination is worse than either alone: [`reachable`] answers "you
/// have an address", and an operator reasonably reads the pair as "other
/// machines can reach me". The address is useless if nothing is listening at
/// the other end of it.
///
/// Not marked a problem when shut. A single-machine install never needs the
/// peer channel, and doctor turning red for an unused feature is how its red
/// stops meaning anything.
#[must_use]
pub fn peer_channel(listening: bool, port: u16) -> Finding {
    if listening {
        Finding::ok("peers", &format!("accepting on {port}"))
    } else {
        Finding {
            line: format!(
                "{:<9} not listening on {port} — other machines cannot join this one",
                "peers:"
            ),
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
    // Placed first because it is the regression this module keeps getting:
    // a check that reports on a port it chose, without saying which.

    /// A machine with an agent on a non-default port must not be told there is
    /// no agent, full stop.
    ///
    /// `doctor` probes `DEFAULT_PORT` and nothing else, so for `agent install
    /// --port 9000` -- or `--no-browser`, which binds no browser port at all --
    /// `listening` is false while the agent is perfectly healthy. The finding
    /// cannot know that, but it CAN say which port it asked about, which is the
    /// difference between a wrong answer and an incomplete one.
    #[test]
    fn a_not_running_agent_says_which_port_was_probed() {
        let f = agent(false, 34333);
        assert!(
            f.line.contains("34333"),
            "the not-running line must name the port it probed, got: {}",
            f.line
        );
        assert!(!f.problem, "no agent is not a fault on an unconfigured box");

        // And it tracks the argument rather than hardcoding the default.
        let f = agent(false, 9000);
        assert!(
            f.line.contains("9000") && !f.line.contains("34333"),
            "got: {}",
            f.line
        );
    }

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

    /// A shut peer channel is REPORTED but is not a problem.
    ///
    /// A single-machine install never needs it, and doctor turning red for an
    /// unused feature is how its red stops meaning anything. It still has to
    /// appear: the whole reason this check exists is that `agent: ok` plus
    /// `network: ok` reads as "other machines can reach me", and neither of
    /// those looks at the port they would actually dial.
    #[test]
    fn a_shut_peer_channel_is_reported_without_being_called_a_problem() {
        let f = peer_channel(false, 34334);
        assert!(!f.problem, "an unused peer channel is not a fault");
        assert!(f.line.contains("34334"), "must name the port: {}", f.line);
        assert!(
            f.line.contains("cannot join"),
            "must say what it costs, not just that a socket is shut: {}",
            f.line
        );
    }

    /// An open one says so, and names the port, so the two findings can be told
    /// apart in a transcript pasted into an issue.
    #[test]
    fn an_open_peer_channel_names_its_port() {
        let f = peer_channel(true, 34334);
        assert!(!f.problem);
        assert!(f.line.contains("34334"), "{}", f.line);
        assert!(
            !f.line.contains("cannot"),
            "an open channel must not read as a shut one: {}",
            f.line
        );
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

#[cfg(test)]
mod disk_tests {
    use super::*;
    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn a_box_with_room_for_an_image_and_a_model_is_fine() {
        for free in [25 * GB, 172 * GB, 4000 * GB] {
            let f = disk_space(free, "/");
            assert!(!f.problem, "{} GB should be fine: {}", free / GB, f.line);
        }
    }

    /// The floor is where it is because of measured sizes, not a round number:
    /// a 2.85 GB image plus a 21.8 GB model is 24.65 GB.
    #[test]
    fn below_an_image_plus_the_smallest_model_is_a_problem() {
        for free in [0, GB, 24 * GB] {
            let f = disk_space(free, "/");
            assert!(
                f.problem,
                "{} GB should be a problem: {}",
                free / GB,
                f.line
            );
        }
    }

    #[test]
    fn the_message_says_what_to_do_and_names_the_path() {
        let f = disk_space(2 * GB, "/mnt/data");
        assert!(f.line.contains("/mnt/data"), "{}", f.line);
        assert!(f.line.contains("2.0 GB"), "the actual figure: {}", f.line);
        assert!(
            f.line.contains("docker image prune"),
            "a remedy: {}",
            f.line
        );
    }

    /// A percentage rule would call this box — 172 GB free, ample — a problem.
    #[test]
    fn a_large_but_full_looking_array_is_judged_on_free_space_not_percent() {
        assert!(!disk_space(172 * GB, "/").problem);
    }
}

#[cfg(test)]
mod disk_caution_tests {
    use super::{disk_caution, disk_space};

    const GB: u64 = 1024 * 1024 * 1024;

    /// The two callers must agree about what "enough" means. They did not have
    /// to before — `doctor` owned the floor and `run` had no check at all — and
    /// a launch begun on a disk `doctor` would refuse is forty minutes spent to
    /// reach a docker error that never mentions space.
    #[test]
    fn run_and_doctor_draw_the_line_in_the_same_place() {
        for free in [0, GB, 24 * GB, 25 * GB, 26 * GB, 400 * GB] {
            assert_eq!(
                disk_caution(free, "/x").is_some(),
                disk_space(free, "/x").problem,
                "they disagree at {free} bytes"
            );
        }
    }

    /// A caution nobody can act on is noise. It has to say the number, where,
    /// and what will actually go wrong — because what the operator sees without
    /// it is a docker error.
    #[test]
    fn the_caution_names_the_space_the_path_and_the_symptom() {
        let why = disk_caution(2 * GB, "/mnt/models").expect("2 GB must caution");
        assert!(why.contains("2.0 GB"), "{why}");
        assert!(why.contains("/mnt/models"), "{why}");
        assert!(why.contains("not about space"), "{why}");
    }

    /// And silence when there is room: a warning that fires on a healthy box is
    /// how an operator learns to ignore this one.
    #[test]
    fn a_roomy_disk_says_nothing() {
        assert!(disk_caution(400 * GB, "/x").is_none());
    }
}
