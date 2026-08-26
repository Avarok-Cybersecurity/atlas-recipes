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
    /// Where to dial it, as typed — a host or an address, optionally with a port.
    pub host: String,
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
        bail!(
            "\"{code}\" is not a join code: it is {} digits, and this needs exactly {}",
            code.chars().filter(char::is_ascii_digit).count(),
            atlasctl_agent::pairing::CODE_DIGITS
        );
    }
    if host.is_empty() {
        bail!("--join names no machine to dial; expected <code>@<host>");
    }
    Ok(Join {
        code: code.to_owned(),
        host: host.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_invitation_splits() {
        let j = parse("12345678@10.10.10.1").expect("parses");
        assert_eq!(j.code, "12345678");
        assert_eq!(j.host, "10.10.10.1");
    }

    #[test]
    fn a_host_and_port_survives() {
        assert_eq!(
            parse("12345678@spark:34334").expect("parses").host,
            "spark:34334"
        );
    }

    /// An IPv6 literal contains colons and a bracketed form is common; the
    /// split must be on the last `@` so the address stays whole.
    #[test]
    fn an_ipv6_literal_is_not_torn_apart() {
        let j = parse("12345678@[fe80::1]:34334").expect("parses");
        assert_eq!(j.host, "[fe80::1]:34334");
    }

    #[test]
    fn surrounding_whitespace_from_a_copy_paste_is_forgiven() {
        assert_eq!(parse("  12345678@host \n").expect("parses").host, "host");
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

    #[test]
    fn an_empty_host_is_refused_rather_than_dialled() {
        assert!(parse("12345678@").is_err());
        assert!(parse("12345678@   ").is_err());
    }
}
