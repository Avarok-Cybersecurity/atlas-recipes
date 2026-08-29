// SPDX-License-Identifier: AGPL-3.0-only

//! Fetching `/metrics` from a model serving on this machine.
//!
//! A hand-written HTTP/1.1 GET rather than an HTTP client crate. The request is
//! one line to loopback, the response is a text body, and the alternative is
//! pulling a TLS stack and its transitive tree into a binary whose reason for
//! existing is a supply-chain compromise — and which must stay statically
//! linkable against musl.
//!
//! Loopback only, deliberately. The address is built here from a port, never
//! taken from a request, so nothing reachable from the browser channel can aim
//! this at another host and use the agent as a scanner.

use anyhow::{Context, Result, bail};
use atlasctl_agent::launchstats::MetricsSource;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

/// How long to wait. A model still loading its weights simply does not answer,
/// and that is the common case rather than an exceptional one.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Largest body accepted, so a misbehaving endpoint cannot make this process
/// grow without bound.
const MAX_BODY: usize = 1 << 20;

/// Reads exposition over loopback HTTP.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpScraper;

/// Reassemble a chunked body.
///
/// Each chunk is a hex length, CRLF, that many bytes, CRLF; a zero length ends
/// the body. A truncated stream is an error rather than a short body, because a
/// half-read exposition looks exactly like an engine that stopped reporting
/// half its metrics.
fn dechunk(body: &str) -> Result<String> {
    let mut rest = body;
    let mut out = String::new();
    loop {
        let (size_line, tail) = rest
            .split_once("\r\n")
            .context("a chunked body ended mid-header")?;
        // A chunk extension after `;` is legal and ignored.
        let hex = size_line.split(';').next().unwrap_or_default().trim();
        let len = usize::from_str_radix(hex, 16)
            .with_context(|| format!("`{hex}` is not a chunk length"))?;
        if len == 0 {
            return Ok(out);
        }
        // `get`, not `[..len]`. The length is a BYTE count off the wire and
        // `tail` is a `&str`, so an index landing mid-character panics rather
        // than erroring — and it reaches here through
        // `String::from_utf8_lossy`, which replaces each invalid byte with a
        // 3-byte U+FFFD and therefore shifts every offset after it. The engine
        // serves ASCII today, so this is a panic waiting on a malformed
        // response rather than one anybody has seen; a scraper thread that
        // dies takes the telemetry with it, silently.
        let (chunk, after) = match (tail.get(..len), tail.get(len..)) {
            (Some(c), Some(a)) => (c, a),
            _ => bail!(
                "a chunked body claimed {len} bytes and sent {} that do not \
                 divide there",
                tail.len()
            ),
        };
        out.push_str(chunk);
        rest = after.strip_prefix("\r\n").unwrap_or(after);
    }
}

impl MetricsSource for HttpScraper {
    fn scrape(&self, port: u16) -> Result<String> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let mut sock = TcpStream::connect_timeout(&addr, TIMEOUT)
            .with_context(|| format!("{addr} is not answering"))?;
        sock.set_read_timeout(Some(TIMEOUT))?;
        sock.set_write_timeout(Some(TIMEOUT))?;

        // `Connection: close` so the read ends at EOF: without it a keep-alive
        // server holds the socket open and the read blocks until the timeout,
        // turning every sample into a two-second stall.
        write!(
            sock,
            "GET /metrics HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/plain\r\nConnection: close\r\n\r\n"
        )
        .context("sending the scrape request")?;
        sock.flush()?;

        let mut raw = Vec::new();
        sock.take(MAX_BODY as u64)
            .read_to_end(&mut raw)
            .context("reading the exposition")?;
        let text = String::from_utf8_lossy(&raw);

        let (head, body) = text
            .split_once("\r\n\r\n")
            .or_else(|| text.split_once("\n\n"))
            .context("the endpoint did not send a complete HTTP response")?;

        let status = head.lines().next().unwrap_or_default();
        if !status.contains(" 200") {
            bail!("the metrics endpoint answered `{}`", status.trim());
        }
        // The engine does send chunked, so this has to decode it. A reader that
        // took the raw framing as the body would parse the hex length lines as
        // metrics and silently lose whatever followed them.
        if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            return dechunk(body);
        }
        Ok(body.to_owned())
    }
}

#[cfg(test)]
mod dechunk_tests {
    use super::dechunk;

    /// The chunk length is a BYTE count off the wire, and the body reaches
    /// `dechunk` as a `&str` through `String::from_utf8_lossy` — which
    /// replaces each invalid byte with a 3-byte U+FFFD and shifts every offset
    /// after it. A length landing mid-character used to panic the scraper
    /// thread, taking the telemetry with it and saying nothing.
    #[test]
    fn a_length_that_lands_mid_character_is_an_error_not_a_panic() {
        // "é" is two bytes, so a claimed length of 1 lands INSIDE it. (A
        // length of 3 would land after "é!" — a valid boundary — which is why
        // the first version of this test decoded happily and proved nothing.)
        let body = "1\r\né\r\n0\r\n\r\n";
        let err = dechunk(body).expect_err("must refuse, and must not panic");
        assert!(
            format!("{err}").contains("divide"),
            "the message must say why: {err}"
        );
    }

    /// The ordinary case the engine actually serves.
    #[test]
    fn an_ascii_chunked_body_still_decodes() {
        let body = "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(dechunk(body).expect("decodes"), "hello world");
    }
}
