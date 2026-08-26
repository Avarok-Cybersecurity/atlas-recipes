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
        if tail.len() < len {
            bail!("a chunked body claimed {len} bytes and sent {}", tail.len());
        }
        out.push_str(&tail[..len]);
        rest = tail[len..].strip_prefix("\r\n").unwrap_or(&tail[len..]);
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
