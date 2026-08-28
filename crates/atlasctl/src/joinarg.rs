// SPDX-License-Identifier: AGPL-3.0-only

//! Parsing the `--join` argument an operator pastes on a new machine.
//!
//! The whole invitation arrives as one token — `12345678@10.10.10.1` — because
//! it is meant to be copied in a single motion from a browser to a terminal,
//! usually on a different machine. Two separate flags would double the chance
//! of one arriving without the other, and half an invitation fails in a way
//! that is tedious to diagnose from the far end.
//!
//! Pure, so the shapes that fail are testable without a network. They matter
//! more than usual: this parse happens on the machine being added, where the
//! operator has the least context and the least appetite for a bad message.

use anyhow::{Result, bail};

/// An invitation to join a fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Join {
    /// The digits minted by the machine doing the inviting.
    pub code: String,
    /// Every place the inviting machine said it can be reached, in the order
    /// it offered them, as typed — a host or address, optionally with a port.
    ///
    /// A list because the inviting machine cannot know which of its networks
    /// the new one shares. A DGX offers its RoCE fabric first (correct, and
    /// fastest, for another DGX) and its ordinary LAN address last; a laptop
    /// can only reach the last. Naming one would work for whichever machine
    /// we guessed and fail on the other, remotely, after a clean install.
    pub hosts: Vec<String>,
}

/// Split `<code>@<host>` into its parts.
///
/// # Errors
/// If the shape is wrong, with a message that shows the expected form rather
/// than only naming the fault.
pub fn parse(raw: &str) -> Result<Join> {
    let raw = raw.trim();
    // Split on the LAST `@`: an IPv6 literal or a future user@host form can
    // contain one, and the code never can.
    let Some((code, host)) = raw.rsplit_once('@') else {
        bail!(
            "expected --join <code>@<host>, for example --join 12345678@10.10.10.1\n\
             (the whole value is shown by the machine you are joining)"
        );
    };
    let code = code.trim();
    let host = host.trim();

    if !atlasctl_agent::joining::looks_like_code(code) {
        // The LENGTH, not the digit count. `looks_like_code` wants
        // `len() == CODE_DIGITS && all ascii_digit`, so a value carrying a
        // non-digit can satisfy the count and still be refused — and the
        // message then read "it is 8 digits, and this needs exactly 8", which
        // is not a complaint anyone can act on. It is read on the machine
        // being added, by someone who pasted something they did not type.
        let len = code.chars().count();
        let stray: String = code.chars().filter(|c| !c.is_ascii_digit()).collect();
        if stray.is_empty() {
            bail!(
                "\"{code}\" is not a join code: it is {len} digits, and this needs exactly {}",
                atlasctl_agent::pairing::CODE_DIGITS
            );
        }
        bail!(
            "\"{code}\" is not a join code: it must be exactly {} digits, and this \
             has {stray:?} in it",
            atlasctl_agent::pairing::CODE_DIGITS
        );
    }
    // Commas separate the alternatives. Neither a hostname, an IPv4 address,
    // a `host:port` nor a bracketed IPv6 literal can contain one, so this
    // cannot split a value that was meant to stay whole.
    let hosts: Vec<String> = host
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .map(str::to_owned)
        .collect();
    if hosts.is_empty() {
        bail!("--join names no machine to dial; expected <code>@<host>");
    }
    Ok(Join {
        code: code.to_owned(),
        hosts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_invitation_splits() {
        let j = parse("12345678@10.10.10.1").expect("parses");
        assert_eq!(j.code, "12345678");
        assert_eq!(j.hosts, ["10.10.10.1"]);
    }

    #[test]
    fn a_host_and_port_survives() {
        assert_eq!(
            parse("12345678@spark:34334").expect("parses").hosts,
            ["spark:34334"]
        );
    }

    /// An IPv6 literal contains colons and a bracketed form is common; the
    /// split must be on the last `@` so the address stays whole.
    #[test]
    fn an_ipv6_literal_is_not_torn_apart() {
        let j = parse("12345678@[fe80::1]:34334").expect("parses");
        assert_eq!(j.hosts, ["[fe80::1]:34334"]);
    }

    #[test]
    fn surrounding_whitespace_from_a_copy_paste_is_forgiven() {
        assert_eq!(parse("  12345678@host \n").expect("parses").hosts, ["host"]);
    }

    /// The message has to teach the shape, because it is read on the machine
    /// being added, by someone who has just pasted something they did not type.
    #[test]
    fn a_missing_at_sign_shows_the_expected_form() {
        let e = parse("12345678").expect_err("must refuse").to_string();
        assert!(e.contains("<code>@<host>"), "{e}");
        assert!(
            e.contains("12345678@"),
            "an example helps more than a rule: {e}"
        );
    }

    #[test]
    fn a_code_of_the_wrong_length_says_so_with_both_numbers() {
        let e = parse("1234@host").expect_err("must refuse").to_string();
        assert!(e.contains("4 digits") || e.contains("is 4"), "{e}");
        assert!(e.contains('8'), "{e}");
    }

    #[test]
    fn a_non_numeric_code_is_refused() {
        assert!(parse("abcdefgh@host").is_err());
    }

    /// The message must not contradict itself. `looks_like_code` wants a
    /// LENGTH of 8 and all-digits; reporting the digit COUNT meant a value
    /// with eight digits and a stray character was refused with "it is 8
    /// digits, and this needs exactly 8".
    #[test]
    fn a_code_with_a_stray_character_is_told_what_is_wrong_with_it() {
        let e = parse("12-345678@host")
            .expect_err("must refuse")
            .to_string();
        assert!(
            !e.contains("it is 8 digits, and this needs exactly 8"),
            "the old message contradicted itself: {e}"
        );
        assert!(e.contains("exactly 8 digits"), "{e}");
        assert!(
            e.contains('-'),
            "naming the offending character is the whole point: {e}"
        );
    }

    /// The plain wrong-length case keeps its original, correct wording.
    #[test]
    fn a_short_all_digit_code_still_reports_its_length() {
        let e = parse("1234@host").expect_err("must refuse").to_string();
        assert!(e.contains("4 digits"), "{e}");
        assert!(e.contains('8'), "{e}");
    }

    #[test]
    fn an_empty_host_is_refused_rather_than_dialled() {
        assert!(parse("12345678@").is_err());
        assert!(parse("12345678@   ").is_err());
        assert!(
            parse("12345678@,,").is_err(),
            "a list of nothing is still nothing to dial"
        );
    }

    /// The case this list exists for: a DGX offers its RoCE fabric first and
    /// its LAN address last, and only the last is reachable from a laptop.
    #[test]
    fn every_alternative_the_inviter_offered_is_kept_in_order() {
        let j = parse("12345678@10.10.10.9,10.10.10.13,192.168.68.68").expect("parses");
        assert_eq!(j.hosts, ["10.10.10.9", "10.10.10.13", "192.168.68.68"]);
    }

    #[test]
    fn a_ragged_list_from_a_copy_paste_still_parses() {
        // Spaces after commas survive a trip through a chat window.
        let j = parse("12345678@ a , b ,, c ").expect("parses");
        assert_eq!(j.hosts, ["a", "b", "c"]);
    }
}
