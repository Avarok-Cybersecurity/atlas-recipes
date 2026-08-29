// SPDX-License-Identifier: AGPL-3.0-only

//! Deciding whether a browser connection may proceed.
//!
//! A loopback port is reachable by every page the user visits: browsers permit
//! cross-origin WebSocket connections to `127.0.0.1`, and the handshake is not
//! subject to a CORS preflight. So the drive-by page — not a network attacker —
//! is the threat this port actually adds, and `Origin` is what defeats it,
//! because browsers set that header from the page's true origin and page script
//! cannot alter it.
//!
//! Everything here is a pure function over headers, so the whole decision is
//! testable without a socket.

/// Origins allowed to drive this agent.
pub const ALLOWED_ORIGINS: &[&str] = &["https://atlasinference.io"];

/// Additional origins for local development, enabled explicitly.
pub const DEV_ORIGINS: &[&str] = &[
    "http://localhost:5173",
    "http://127.0.0.1:5173",
    "http://localhost:4173",
    "http://127.0.0.1:4173",
];

/// Host values a loopback connection may legitimately carry.
///
/// This is the anti-DNS-rebinding control. Rebinding changes what a name
/// *resolves to*, not what the browser puts in `Host`, so a page served from
/// `attacker.com` still sends `Host: attacker.com:34333` and is refused here
/// even if its name now points at 127.0.0.1.
fn host_allowed(host: &str, port: u16) -> bool {
    let expected = [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ];
    expected.iter().any(|e| e == host)
}

/// Why a connection was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No `Origin` header at all.
    #[error("no Origin header; refusing a connection whose origin cannot be established")]
    MissingOrigin,

    /// An origin outside the allowlist.
    #[error("origin `{0}` is not allowed to drive this agent")]
    ForeignOrigin(String),

    /// No `Host` header.
    #[error("no Host header")]
    MissingHost,

    /// A `Host` that is not this loopback listener.
    #[error("host `{0}` is not this agent's loopback address (possible DNS rebinding)")]
    ForeignHost(String),
}

/// Decide whether a connection may upgrade.
///
/// Matching is exact string equality. Prefix or substring matching would accept
/// `https://atlasinference.io.evil.com`, and a scheme-insensitive match would
/// accept plain `http://atlasinference.io`; neither is our origin.
pub fn check(
    origin: Option<&str>,
    host: Option<&str>,
    port: u16,
    allow_dev: bool,
) -> Result<(), Refusal> {
    let host = host.ok_or(Refusal::MissingHost)?;
    if !host_allowed(host, port) {
        return Err(Refusal::ForeignHost(host.to_string()));
    }

    // A missing Origin is refused rather than trusted. Browsers always send one
    // for a WebSocket handshake, so its absence means the peer is not the thing
    // this port exists to serve.
    let origin = origin.ok_or(Refusal::MissingOrigin)?;
    let allowed = ALLOWED_ORIGINS.iter().chain(if allow_dev {
        DEV_ORIGINS.iter()
    } else {
        [].iter()
    });
    if allowed.into_iter().any(|a| *a == origin) {
        Ok(())
    } else {
        Err(Refusal::ForeignOrigin(origin.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 34333;

    fn ok(origin: &str) -> Result<(), Refusal> {
        check(
            Some(origin),
            Some(&format!("127.0.0.1:{PORT}")),
            PORT,
            false,
        )
    }

    #[test]
    fn the_real_site_is_allowed() {
        assert!(ok("https://atlasinference.io").is_ok());
    }

    #[test]
    fn a_lookalike_origin_is_refused() {
        // Substring or prefix matching would accept every one of these.
        for evil in [
            "https://atlasinference.io.evil.com",
            "https://evil.com/atlasinference.io",
            "https://atlasinference.io.",
            "https://notatlasinference.io",
            "https://sub.atlasinference.io",
        ] {
            assert!(ok(evil).is_err(), "{evil} must be refused");
        }
    }

    #[test]
    fn the_scheme_and_port_must_match_exactly() {
        assert!(
            ok("http://atlasinference.io").is_err(),
            "plain http is not our origin"
        );
        assert!(
            ok("https://atlasinference.io:8443").is_err(),
            "a different port is a different origin"
        );
    }

    #[test]
    fn a_missing_or_null_origin_is_refused() {
        assert_eq!(
            check(None, Some(&format!("127.0.0.1:{PORT}")), PORT, false),
            Err(Refusal::MissingOrigin)
        );
        // `null` is what a sandboxed iframe or a file:// page sends.
        assert!(ok("null").is_err());
    }

    #[test]
    fn dns_rebinding_is_refused_on_the_host_header() {
        // The attacker's page keeps its own Origin *and* its own Host, so this
        // fails twice over; assert the Host check specifically.
        let r = check(
            Some("https://atlasinference.io"),
            Some("attacker.com:34333"),
            PORT,
            false,
        );
        assert_eq!(r, Err(Refusal::ForeignHost("attacker.com:34333".into())));
    }

    #[test]
    fn every_loopback_spelling_of_host_is_accepted() {
        for h in ["127.0.0.1:34333", "localhost:34333", "[::1]:34333"] {
            assert!(
                check(Some("https://atlasinference.io"), Some(h), PORT, false).is_ok(),
                "{h} should be accepted"
            );
        }
    }

    #[test]
    fn a_host_on_the_wrong_port_is_refused() {
        assert!(
            check(
                Some("https://atlasinference.io"),
                Some("127.0.0.1:1234"),
                PORT,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn dev_origins_are_off_unless_asked_for() {
        let host = format!("127.0.0.1:{PORT}");
        assert!(check(Some("http://localhost:5173"), Some(&host), PORT, false).is_err());
        assert!(check(Some("http://localhost:5173"), Some(&host), PORT, true).is_ok());
        // Even with dev origins on, an unrelated origin stays refused.
        assert!(check(Some("https://evil.com"), Some(&host), PORT, true).is_err());
    }

    #[test]
    fn a_missing_host_is_refused() {
        assert_eq!(
            check(Some("https://atlasinference.io"), None, PORT, false),
            Err(Refusal::MissingHost)
        );
    }
}
