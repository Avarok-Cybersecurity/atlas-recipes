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
        launcher: std::sync::Arc::new(RecordingLauncher::new()),
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

/// A launcher that blocks for a fixed time, standing in for `docker run`.
struct SleepyLauncher {
    delay: std::time::Duration,
}

impl crate::launcher::Launcher for SleepyLauncher {
    fn preview(
        &self,
        _: &atlasctl_core::Recipe,
        _: &std::collections::BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Preview, atlasctl_protocol::msg::AgentError> {
        Err(atlasctl_protocol::msg::AgentError::NotReady)
    }
    fn launch(
        &self,
        _: &atlasctl_core::Recipe,
        _: &std::collections::BTreeMap<String, atlasctl_core::ScalarValue>,
    ) -> Result<crate::launcher::Started, atlasctl_protocol::msg::AgentError> {
        // Exactly what the real launcher does to the calling thread.
        std::thread::sleep(self.delay);
        Ok(crate::launcher::Started {
            container: "atlas-sleepy".to_owned(),
            endpoint: None,
        })
    }
    fn stop(&self, _: &str) -> Result<(), atlasctl_protocol::msg::AgentError> {
        Ok(())
    }
    fn running(
        &self,
    ) -> Result<Vec<atlasctl_protocol::msg::RunningLaunch>, atlasctl_protocol::msg::AgentError>
    {
        Ok(Vec::new())
    }
}

/// A launch must not park the runtime's only worker.
///
/// `Launcher::launch` shells out to `docker run` and does not return until the
/// container is up — seconds, or minutes behind an image pull. Run inline on the
/// websocket task, that parks a runtime worker for the whole launch, and the
/// cost lands on everything else sharing the runtime: other sessions, and the
/// timers that are supposed to BOUND these operations.
///
/// The launch runs inside `tokio::spawn` here, not in the test body. That
/// distinction is the entire test: `#[tokio::test]` drives the body with
/// `block_on` on the *main* thread, so blocking there leaves the worker free and
/// the timer fires on time with or without the fix. An earlier version of this
/// test did exactly that and passed against the unfixed code — it proved
/// nothing. Only work spawned onto the worker can starve it.
///
/// `worker_threads = 1` makes the starvation deterministic rather than a race.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_launch_does_not_park_the_only_worker() {
    use std::time::{Duration, Instant};
    const LAUNCH: Duration = Duration::from_millis(900);
    const TIMER: Duration = Duration::from_millis(50);

    // Registered before the launch begins, so it is genuinely waiting on the
    // runtime rather than being created after the block started.
    let timer = tokio::spawn(async move {
        let t = Instant::now();
        tokio::time::sleep(TIMER).await;
        t.elapsed()
    });
    tokio::task::yield_now().await;

    // Owns everything it borrows, so the task is 'static and runs ON the worker
    // — which is where the real websocket loop calls `handle`.
    let launch = tokio::spawn(async move {
        let registry = RegistrySet::builtin_only();
        let launcher = std::sync::Arc::new(SleepyLauncher { delay: LAUNCH });
        let (mut session, _welcome) = Session::new(SessionDeps {
            accelerator: "",
            registry: &registry,
            launcher,
            token: TOKEN,
            can_launch: Ok(()),
            fleet: None,
            cluster: None,
            telemetry: None,
            joining: None,
            relay: None,
        });
        let out = session
            .handle(ClientMsg::Hello {
                protocol_version: atlasctl_protocol::PROTOCOL_VERSION,
                token: TOKEN.into(),
            })
            .await;
        assert!(
            matches!(out[0], ServerMsg::Ready { .. }),
            "handshake failed: {out:?}"
        );
        let t0 = Instant::now();
        let _ = session
            .handle(ClientMsg::Launch {
                id: 1,
                recipe: atlasctl_protocol::RecipeId::parse("qwen3.6-27b-fp8").expect("valid id"),
                settings: std::collections::BTreeMap::new(),
                on: None,
            })
            .await;
        t0.elapsed()
    });

    let launch_took = launch.await.expect("launch task");
    let timer_took = timer.await.expect("timer task");

    // Guards against a pass earned by the launcher never blocking at all.
    assert!(
        launch_took >= LAUNCH,
        "the launcher did not actually block ({launch_took:?}); this test would prove nothing"
    );
    assert!(
        timer_took < LAUNCH / 2,
        "a {TIMER:?} timer took {timer_took:?} to fire: the launch parked the runtime's only \
         worker, so nothing else — including the timeouts meant to bound it — could run"
    );
}
