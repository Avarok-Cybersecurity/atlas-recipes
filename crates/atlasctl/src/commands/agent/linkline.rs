// SPDX-License-Identifier: AGPL-3.0-only

//! What an agent says about this machine's network on startup.
//!
//! Split from [`super`] for size, along the seam that was already there: this
//! is the one pure decision in a function that is otherwise all I/O, and it was
//! already carrying its own test module for exactly that reason.

/// What to tell the operator about this machine's links, as three distinct
/// facts rather than two.
///
/// The distinction is the whole point: "we could not look" and "we looked and
/// there is nothing" send a person to different places, and only one of them
/// is a statement about their network. Pure so it can be tested — the
/// enumeration it describes shells out, which is why the claim it makes is
/// worth pinning down separately from the I/O that feeds it.
pub(super) fn link_line(first: Result<Option<(String, &'static str)>, String>) -> String {
    match first {
        Err(why) => format!(
            "could not read this machine's network interfaces: {why} \
             — clustering is off until that is fixed; run `atlasctl doctor`"
        ),
        Ok(None) => "no usable network link — this agent cannot take part in a cluster".to_owned(),
        Ok(Some((addr, class))) => format!("cluster address: {addr} ({class})"),
    }
}

#[cfg(test)]
mod link_line_tests {
    use super::link_line;

    #[test]
    fn a_failed_enumeration_is_never_reported_as_an_absent_network() {
        let e = link_line(Err("running `ip -o -4 addr show`: No such file".into()));
        assert!(
            e.contains("could not read"),
            "the operator must learn we could not look: {e}"
        );
        assert!(
            !e.contains("no usable network link"),
            "claiming the network is absent when `ip` is missing sends them to \
             debug hardware that is fine: {e}"
        );
        assert!(
            e.contains("doctor"),
            "and it must say what to run next: {e}"
        );
    }

    #[test]
    fn an_empty_enumeration_still_says_the_network_is_the_problem() {
        // The other side of the same coin: when we DID look and found nothing,
        // hedging would be just as wrong.
        let e = link_line(Ok(None));
        assert_eq!(
            e,
            "no usable network link — this agent cannot take part in a cluster"
        );
    }

    #[test]
    fn a_found_address_is_reported_with_its_link_class() {
        // The class is what tells RoCE from Wi-Fi, which is what the operator
        // needs to know a cluster will actually be fast.
        assert_eq!(
            link_line(Ok(Some(("10.10.10.1".to_owned(), "InfiniBand")))),
            "cluster address: 10.10.10.1 (InfiniBand)"
        );
    }
}
