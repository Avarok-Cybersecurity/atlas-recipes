// SPDX-License-Identifier: AGPL-3.0-only

//! Real TLS handshakes between two identities, over a loopback socket.
//!
//! These are the tests that must not rot: they assert that trust is decided by
//! the pin store and by nothing else.

use super::tls::{
    PinnedPeerVerifier, certificate_for, client_config, peer_identity, server_config,
};
use crate::identity::{Identity, Pin, PinStore};
use atlasctl_protocol::fleet::DisplayName;
use ed25519_dalek::ed25519::pkcs8::EncodePrivateKey;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-tls-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&p).expect("scratch");
        Self(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pin_of(store: &PinStore, who: &Identity) {
    store
        .add(Pin {
            id: who.id(),
            public_key: hex::encode(who.public().as_bytes()),
            name: DisplayName::new("peer"),
            paired_at: 0,
            last_address: None,
            controller: false,
        })
        .expect("pin");
}

/// Run one mutually-authenticated handshake and return whether it completed.
async fn handshake(
    server: &Identity,
    server_pins: PinStore,
    client: &Identity,
    client_pins: PinStore,
    expect: Option<atlasctl_protocol::fleet::NodeId>,
) -> anyhow::Result<()> {
    let scfg = server_config(server, PinnedPeerVerifier::pinned(server_pins, None))?;
    let ccfg = client_config(client, PinnedPeerVerifier::pinned(client_pins, expect))?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let acceptor = TlsAcceptor::from(Arc::new(scfg));

    let server_side = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        let mut tls = acceptor.accept(tcp).await?;
        tls.write_all(b"ok").await?;
        tls.flush().await?;
        Ok::<_, anyhow::Error>(())
    });

    let connector = TlsConnector::from(Arc::new(ccfg));
    let tcp = TcpStream::connect(addr).await?;
    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")?.to_owned();
    let mut tls = connector.connect(name, tcp).await?;
    let mut buf = [0u8; 2];
    tls.read_exact(&mut buf).await?;
    anyhow::ensure!(&buf == b"ok");
    server_side.await??;
    Ok(())
}

/// Same as `handshake`, but the listener uses the join-window gate rather than
/// refusing every unpinned peer outright.
async fn handshake_while(
    server: &Identity,
    server_pins: PinStore,
    client: &Identity,
    client_pins: PinStore,
    gate: Arc<dyn Fn() -> bool + Send + Sync>,
) -> anyhow::Result<()> {
    let scfg = server_config(server, PinnedPeerVerifier::while_joining(server_pins, gate))?;
    // The joining side has a code, not a pin, so it accepts an unpinned server.
    let ccfg = client_config(client, PinnedPeerVerifier::pairing(client_pins))?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let acceptor = TlsAcceptor::from(Arc::new(scfg));

    let server_side = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        let mut tls = acceptor.accept(tcp).await?;
        tls.write_all(b"ok").await?;
        tls.flush().await?;
        Ok::<_, anyhow::Error>(())
    });

    let connector = TlsConnector::from(Arc::new(ccfg));
    let tcp = TcpStream::connect(addr).await?;
    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")?.to_owned();
    let mut tls = connector.connect(name, tcp).await?;
    let mut buf = [0u8; 2];
    tls.read_exact(&mut buf).await?;
    anyhow::ensure!(&buf == b"ok");
    server_side.await??;
    Ok(())
}

/// A machine being onboarded is unpinned by definition, so the listener has to
/// let it reach the ceremony — but only while a human has a join code
/// outstanding.
#[tokio::test]
async fn an_unpinned_agent_is_admitted_while_a_join_is_pending() {
    let ta = Tmp::new("ja");
    let tb = Tmp::new("jb");
    let joining = Identity::generate();
    let host = Identity::generate();

    handshake_while(
        &host,
        PinStore::new(&tb.0),
        &joining,
        PinStore::new(&ta.0),
        Arc::new(|| true),
    )
    .await
    .expect("a pending join must admit an unpinned peer");
}

/// And the window is the whole point: with no code outstanding the stranger is
/// refused during the handshake, so for all the time nobody is onboarding —
/// which is almost all of it — an unpaired machine reaches no further than
/// rustls' ClientHello handling.
#[tokio::test]
async fn the_same_agent_is_refused_once_the_window_closes() {
    let ta = Tmp::new("jc");
    let tb = Tmp::new("jd");
    let joining = Identity::generate();
    let host = Identity::generate();

    let err = handshake_while(
        &host,
        PinStore::new(&tb.0),
        &joining,
        PinStore::new(&ta.0),
        Arc::new(|| false),
    )
    .await
    .expect_err("no pending join must mean no session");
    let msg = err.to_string();
    assert!(
        msg.contains("not paired") || msg.contains("certificate") || msg.contains("alert"),
        "unexpected failure: {msg}"
    );
}

/// The gate is read per handshake, not captured once, or a code that has been
/// used or expired would keep letting strangers in.
#[tokio::test]
async fn the_gate_is_consulted_on_every_handshake() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let open = Arc::new(AtomicBool::new(true));
    let g = Arc::clone(&open);
    let gate: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || g.load(Ordering::SeqCst));

    let ta = Tmp::new("je");
    let tb = Tmp::new("jf");
    let joining = Identity::generate();
    let host = Identity::generate();

    handshake_while(
        &host,
        PinStore::new(&tb.0),
        &joining,
        PinStore::new(&ta.0),
        Arc::clone(&gate),
    )
    .await
    .expect("open");

    open.store(false, Ordering::SeqCst);

    handshake_while(
        &host,
        PinStore::new(&tb.0),
        &joining,
        PinStore::new(&ta.0),
        gate,
    )
    .await
    .expect_err("the same verifier must refuse once the window has closed");
}

#[test]
fn the_certificate_carries_the_identity_key_so_pinning_a_key_pins_the_node() {
    let me = Identity::generate();
    let pc = certificate_for(&me).expect("cert");
    let (id, key) = peer_identity(&pc.cert).expect("recovers the key");
    assert_eq!(id, me.id(), "the cert's key must be the identity key");
    assert_eq!(key.as_bytes(), me.public().as_bytes());
}

#[test]
fn a_regenerated_certificate_keeps_the_same_identity() {
    // Certs may be rebuilt at will; pairings must survive it. A trust model
    // that broke on rollover would teach people to click through the check.
    let me = Identity::generate();
    let a = certificate_for(&me).expect("cert a");
    let b = certificate_for(&me).expect("cert b");
    assert_eq!(
        peer_identity(&a.cert).expect("a").0,
        peer_identity(&b.cert).expect("b").0
    );
}

#[tokio::test]
async fn two_paired_agents_complete_a_mutually_authenticated_handshake() {
    let ta = Tmp::new("a");
    let tb = Tmp::new("b");
    let a = Identity::generate();
    let b = Identity::generate();
    let apins = PinStore::new(&ta.0);
    let bpins = PinStore::new(&tb.0);
    pin_of(&apins, &b);
    pin_of(&bpins, &a);

    handshake(&a, apins, &b, bpins, Some(a.id()))
        .await
        .expect("paired peers must connect");
}

#[tokio::test]
async fn an_unpaired_agent_is_refused_by_the_listener() {
    // Discovery grants no authority: knowing the address is not permission.
    let ta = Tmp::new("ua");
    let tb = Tmp::new("ub");
    let a = Identity::generate();
    let b = Identity::generate();
    let apins = PinStore::new(&ta.0);
    let bpins = PinStore::new(&tb.0);
    // The client trusts the server, but the server has never paired the client.
    pin_of(&bpins, &a);

    let err = handshake(&a, apins, &b, bpins, Some(a.id()))
        .await
        .expect_err("an unpaired client must not get a session");
    let msg = err.to_string();
    assert!(
        msg.contains("not paired") || msg.contains("certificate") || msg.contains("alert"),
        "unexpected failure: {msg}"
    );
}

#[tokio::test]
async fn a_client_that_reaches_the_wrong_node_refuses_it() {
    // Dialling 10.10.10.2 and getting some other agent must be an error, not a
    // silent success — otherwise a DNS or ARP trick silently redirects a launch.
    let ta = Tmp::new("wa");
    let tb = Tmp::new("wb");
    let a = Identity::generate();
    let b = Identity::generate();
    let elsewhere = Identity::generate();
    let apins = PinStore::new(&ta.0);
    let bpins = PinStore::new(&tb.0);
    pin_of(&apins, &b);
    pin_of(&bpins, &a);

    let err = handshake(&a, apins, &b, bpins, Some(elsewhere.id()))
        .await
        .expect_err("connecting to the wrong node must fail");
    assert!(err.to_string().contains("expected") || err.to_string().contains("alert"));
}

#[tokio::test]
async fn revoking_a_pin_takes_effect_on_the_next_connection() {
    // `atlasctl peer remove` that needed a restart would not be a revocation.
    let ta = Tmp::new("ra");
    let tb = Tmp::new("rb");
    let a = Identity::generate();
    let b = Identity::generate();
    let apins = PinStore::new(&ta.0);
    let bpins = PinStore::new(&tb.0);
    pin_of(&apins, &b);
    pin_of(&bpins, &a);

    handshake(&a, apins.clone(), &b, bpins.clone(), Some(a.id()))
        .await
        .expect("paired");

    assert!(apins.remove(b.id()).expect("removes"));
    // b is the client here; a is the listener and no longer trusts b.
    handshake(&a, apins, &b, bpins, Some(a.id()))
        .await
        .expect_err("a removed pin must be refused immediately");
}

/// A certificate carrying a second Ed25519 SPKI must be refused outright.
///
/// This is the parser differential that `peer_identity`'s uniqueness check
/// exists to close, and it is worth spelling out because the certificate is
/// otherwise perfectly valid and rustls accepts it.
///
/// rustls verifies the handshake signature against the certificate's *real*
/// SubjectPublicKeyInfo. `peer_identity` finds a key by scanning DER bytes. An
/// attacker who signs with their own key, but embeds a victim's public key
/// earlier in the certificate — the serial number precedes the SPKI, and its
/// bytes are arbitrary — makes the two disagree. Taking the first match would
/// report the victim's identity for a connection only the attacker can produce,
/// which on the pinned fast path is admission to the fleet as that victim.
#[test]
fn a_certificate_with_a_smuggled_second_key_is_refused_not_guessed() {
    let attacker = Identity::generate();
    let victim = Identity::generate();

    // The exact 44-byte SPKI the victim's own certificate would carry.
    let mut smuggled = vec![
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    smuggled.extend_from_slice(victim.public().as_bytes());

    let pkcs8 = attacker
        .signing_key()
        .to_pkcs8_der()
        .expect("encode attacker key");
    let key_pair = rcgen::KeyPair::try_from(pkcs8.as_bytes()).expect("rcgen key");
    let mut params =
        rcgen::CertificateParams::new(vec![attacker.id().to_string()]).expect("params");
    // The serial number is arbitrary bytes and is emitted before the SPKI.
    params.serial_number = Some(rcgen::SerialNumber::from_slice(&smuggled));
    let cert = params.self_signed(&key_pair).expect("self-sign");
    let der = cert.der().clone();

    // Precondition: the smuggled key really is in there, ahead of the real one.
    let hay = der.as_ref();
    let first = hay
        .windows(smuggled.len())
        .position(|w| w == smuggled.as_slice())
        .expect("the victim key must actually be embedded for this test to mean anything");
    let real = {
        let mut probe = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        probe.extend_from_slice(attacker.public().as_bytes());
        hay.windows(probe.len())
            .position(|w| w == probe.as_slice())
            .expect("the attacker's own key is present")
    };
    assert!(
        first < real,
        "the smuggled key must precede the real one, or the test proves nothing"
    );

    let err = peer_identity(&der).expect_err("an ambiguous certificate must be refused");
    let msg = format!("{err}");
    assert!(
        msg.contains("more than one"),
        "the refusal must say why it is ambiguous, got: {msg}"
    );
}
