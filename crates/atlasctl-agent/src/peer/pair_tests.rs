// SPDX-License-Identifier: AGPL-3.0-only

//! The ceremony over a real TLS connection between two real identities.

use super::pair::{Paired, Role, run};
use super::tls::{PinnedPeerVerifier, client_config, peer_identity, server_config};
use crate::identity::{Identity, PinStore};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-pair-{tag}-{}-{}",
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

/// Run a full ceremony between two fresh agents over loopback TLS.
///
/// Returns both sides' outcomes so a test can assert they agree.
async fn ceremony(initiator_code: &str, responder_code: &str) -> (Result<Paired>, Result<Paired>) {
    let ta = Tmp::new("i");
    let tb = Tmp::new("r");
    let initiator = Identity::generate();
    let responder = Identity::generate();
    let init_id = initiator.id();
    let resp_id = responder.id();

    // Pairing mode on both sides: neither has pinned the other yet, so the
    // certificate cannot be what authenticates. The PAKE is.
    let scfg = server_config(
        &responder,
        PinnedPeerVerifier::pairing(PinStore::new(&tb.0)),
    )
    .expect("server config");
    let ccfg = client_config(
        &initiator,
        PinnedPeerVerifier::pairing(PinStore::new(&ta.0)),
    )
    .expect("client config");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let acceptor = TlsAcceptor::from(Arc::new(scfg));

    let rcode = responder_code.to_owned();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await?;
        let mut tls = acceptor.accept(tcp).await?;
        let (_, conn) = tls.get_ref();
        let peer_cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no client certificate"))?;
        let (peer, _) = peer_identity(&peer_cert)?;
        let binding = crate::pairing::binding_from_server(conn)?;
        run(&mut tls, Role::Responder, &responder, peer, &rcode, binding).await
    });

    let connector = TlsConnector::from(Arc::new(ccfg));
    let tcp = TcpStream::connect(addr).await.expect("connect");
    let name = rustls::pki_types::ServerName::try_from("peer.atlas.invalid")
        .expect("name")
        .to_owned();
    let client_out = async {
        let mut tls = connector.connect(name, tcp).await?;
        let (_, conn) = tls.get_ref();
        let peer_cert = conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no server certificate"))?;
        let (peer, _) = peer_identity(&peer_cert)?;
        let binding = crate::pairing::binding_from_client(conn)?;
        run(
            &mut tls,
            Role::Initiator,
            &initiator,
            peer,
            initiator_code,
            binding,
        )
        .await
    }
    .await;

    let server_out = server.await.expect("server task");
    // Sanity: the transport really did see two distinct identities.
    assert_ne!(init_id, resp_id);
    (client_out, server_out)
}

#[tokio::test]
async fn two_agents_with_the_same_code_pair_and_agree_on_the_words() {
    let (i, r) = ceremony("13572468", "13572468").await;
    let i = i.expect("initiator pairs");
    let r = r.expect("responder pairs");

    // Both humans must see the same words, or the comparison is theatre.
    assert_eq!(i.verification, r.verification);
    assert_eq!(i.verification.len(), 9);

    // Each side learned the other's real key.
    crate::identity::verify_key_matches(i.node, &i.public_key)
        .expect("initiator got the responder's real key");
    crate::identity::verify_key_matches(r.node, &r.public_key)
        .expect("responder got the initiator's real key");
    assert_ne!(i.node, r.node);
}

#[tokio::test]
async fn a_wrong_code_pairs_nobody() {
    let (i, r) = ceremony("13572468", "13572469").await;
    let ie = i.expect_err("a wrong code must not pair");
    assert!(
        ie.to_string().contains("key confirmation failed") || ie.to_string().contains("refused"),
        "unexpected: {ie}"
    );
    assert!(r.is_err(), "neither side may end up trusting the other");
}

#[tokio::test]
async fn a_malformed_code_never_reaches_the_network() {
    let (i, _r) = ceremony("12", "12").await;
    let e = i.expect_err("a two-digit code is not a code");
    assert!(e.to_string().contains("8 digits"));
}

#[test]
fn a_peer_cannot_ask_to_be_pinned_under_a_key_it_does_not_hold() {
    // The last check in the ceremony. Without it, a peer could authenticate
    // with one key and hand over another to be recorded, pinning an identity
    // nobody ever proved.
    let real = Identity::generate();
    let other = Identity::generate();
    crate::identity::verify_key_matches(real.id(), &hex::encode(real.public().as_bytes()))
        .expect("its own key verifies");
    let err =
        crate::identity::verify_key_matches(real.id(), &hex::encode(other.public().as_bytes()))
            .expect_err("a substituted key must be refused");
    assert!(err.to_string().contains("does not match"));
}
