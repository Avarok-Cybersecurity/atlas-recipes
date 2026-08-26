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
