// SPDX-License-Identifier: AGPL-3.0-only

//! Transport-level tests, over a real socket on an ephemeral loopback port.

use super::*;
use crate::launcher::RecordingLauncher;
use std::sync::Arc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn state(port: u16) -> Arc<AgentState> {
    Arc::new(AgentState {
        accelerator: String::new(),
        registry: RegistrySet::builtin_only(),
        launcher: Box::new(RecordingLauncher::new()),
        token: TOKEN.to_string(),
        can_launch: Ok(()),
        joining: None,
        relay: None,
        fleet: None,
        cluster: None,
        telemetry: None,
        events: tokio::sync::broadcast::channel(8).0,
        port,
        allow_dev_origins: false,
    })
}

/// Start the listener on an ephemeral port and return its address.
async fn spawn() -> SocketAddr {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let app = router(state(addr.port()));
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    // Let the accept loop start.
    tokio::task::yield_now().await;
    addr
}

/// Perform the HTTP upgrade by hand, so the response to a refused handshake can
/// be inspected — a websocket client library would just report a failure.
async fn handshake(addr: SocketAddr, origin: Option<&str>, host: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let host = host
        .map(str::to_string)
        .unwrap_or_else(|| format!("127.0.0.1:{}", addr.port()));

    let mut req = format!(
        "GET /ws HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    if let Some(o) = origin {
        req.push_str(&format!("Origin: {o}\r\n"));
    }
    req.push_str("\r\n");

    stream.write_all(req.as_bytes()).await.expect("write");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).to_string()
}

#[tokio::test]
async fn the_listener_binds_loopback_only() {
    let addr = spawn().await;
    assert!(
        addr.ip().is_loopback(),
        "the browser channel must never be network-reachable"
    );
}

#[tokio::test]
async fn the_real_site_completes_the_upgrade() {
    let addr = spawn().await;
    let resp = handshake(addr, Some("https://atlasinference.io"), None).await;
    assert!(
        resp.starts_with("HTTP/1.1 101"),
        "expected an upgrade, got: {resp}"
    );
}

#[tokio::test]
async fn a_hostile_origin_is_refused_before_any_websocket_exists() {
    let addr = spawn().await;
    for evil in [
        "https://evil.com",
        "https://atlasinference.io.evil.com",
        "http://atlasinference.io",
        "null",
    ] {
        let resp = handshake(addr, Some(evil), None).await;
        assert!(
            resp.starts_with("HTTP/1.1 403"),
            "{evil} should be refused, got: {resp}"
        );
        assert!(!resp.contains("101"), "{evil} must not be upgraded");
    }
}

#[tokio::test]
async fn a_connection_with_no_origin_is_refused() {
    let addr = spawn().await;
    let resp = handshake(addr, None, None).await;
    assert!(resp.starts_with("HTTP/1.1 403"), "got: {resp}");
}

#[tokio::test]
async fn a_rebound_host_header_is_refused_even_with_a_valid_origin() {
    // The DNS-rebinding case: the attacker's name now resolves to 127.0.0.1,
    // but the browser still sends the attacker's Host.
    let addr = spawn().await;
    let resp = handshake(
        addr,
        Some("https://atlasinference.io"),
        Some("attacker.com:1234"),
    )
    .await;
    assert!(resp.starts_with("HTTP/1.1 403"), "got: {resp}");
}

#[tokio::test]
async fn dev_origins_are_refused_unless_enabled() {
    let addr = spawn().await;
    let resp = handshake(addr, Some("http://localhost:5173"), None).await;
    assert!(
        resp.starts_with("HTTP/1.1 403"),
        "dev origins are off by default, got: {resp}"
    );
}
