// SPDX-License-Identifier: AGPL-3.0-only

//! TLS for the peer channel: mutual authentication, pinned to keys.
//!
//! The certificate is an envelope and nothing more. Both sides present a
//! self-signed certificate whose subject public key **is** the agent's Ed25519
//! identity key, and both sides verify their peer by hashing that key and
//! checking the fingerprint against the pin store. Expiry, subject names, and
//! chains of trust play no part: there is no CA, and there is deliberately no
//! way to add one.
//!
//! That choice is the point. The two obvious alternatives are worse:
//!
//! * A **locally generated root CA** installed into the OS trust store hands
//!   the machine a permanent, machine-wide interception capability that
//!   outlives this software and is only as safe as wherever its key ended up.
//!   That is the Superfish / eDellRoot failure, and shipping it to close a gap
//!   on a private link would be a net increase in the user's exposure.
//! * A **cluster CA** makes its private key a crown jewel with a distribution
//!   and rotation story nobody operates correctly, and buys no assurance over
//!   pinning the key you already verified by hand.
//!
//! Pinning the raw key also means a certificate may be regenerated freely — on
//! every boot, if we like — without breaking a single pairing. A trust model
//! that broke on cert rollover would train people to re-pair without checking
//! the fingerprint, which defeats the ceremony it exists to protect.

use crate::identity::{Identity, PinStore, fingerprint};
use anyhow::{Context, Result};
use atlasctl_protocol::fleet::NodeId;
use ed25519_dalek::VerifyingKey;
use ed25519_dalek::ed25519::pkcs8::EncodePrivateKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use std::sync::Arc;

/// Minimum TLS version. Nothing below 1.3 is offered or accepted.
const PROTOCOL_VERSIONS: &[&rustls::SupportedProtocolVersion] = &[&rustls::version::TLS13];

/// A certificate and key derived from this agent's identity.
pub struct PeerCertificate {
    /// DER certificate.
    pub cert: CertificateDer<'static>,
    /// DER private key.
    pub key: PrivateKeyDer<'static>,
}

impl std::fmt::Debug for PeerCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerCertificate").finish_non_exhaustive()
    }
}

/// Build a self-signed certificate whose public key is the agent's identity.
///
/// The subject name is the node fingerprint, which nothing verifies — it is
/// there so that a human running `openssl x509 -text` sees which node they are
/// looking at.
///
/// # Errors
/// If the identity key cannot be encoded, or the certificate cannot be built.
pub fn certificate_for(identity: &Identity) -> Result<PeerCertificate> {
    let pkcs8 = identity
        .signing_key()
        .to_pkcs8_der()
        .context("encoding the identity key as PKCS#8")?;
    let key_pair = rcgen::KeyPair::try_from(pkcs8.as_bytes())
        .context("rcgen could not use the identity key")?;

    let mut params = rcgen::CertificateParams::new(vec![identity.id().to_string()])
        .context("building certificate parameters")?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, identity.id().to_string());

    let cert = params
        .self_signed(&key_pair)
        .context("self-signing the peer certificate")?;

    Ok(PeerCertificate {
        cert: cert.der().clone(),
        key: PrivateKeyDer::try_from(key_pair.serialize_der())
            .map_err(|e| anyhow::anyhow!("serialising the private key: {e}"))?,
    })
}

/// Recover the Ed25519 public key from a peer's certificate, and thus its id.
///
/// Reads the SubjectPublicKeyInfo without a full X.509 parser: an Ed25519 SPKI
/// is a fixed 44-byte structure ending in the 32-byte key, and matching that
/// exact prefix is both simpler and stricter than parsing — a certificate
/// carrying any other algorithm fails to match rather than being accepted with
/// a key we then misinterpret.
///
/// # Errors
/// If the certificate does not contain exactly one Ed25519 SPKI.
pub fn peer_identity(cert: &CertificateDer<'_>) -> Result<(NodeId, VerifyingKey)> {
    /// DER prefix of a SubjectPublicKeyInfo for Ed25519 (RFC 8410):
    /// SEQUENCE { SEQUENCE { OID 1.3.101.112 }, BIT STRING (32 bytes) }
    const SPKI_PREFIX: [u8; 12] = [
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];

    let der = cert.as_ref();
    let start = der
        .windows(SPKI_PREFIX.len())
        .position(|w| w == SPKI_PREFIX)
        .context("certificate carries no Ed25519 public key")?;
    let key_start = start + SPKI_PREFIX.len();
    let bytes: [u8; 32] = der
        .get(key_start..key_start + 32)
        .context("certificate is truncated inside its public key")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key is not 32 bytes"))?;
    let public =
        VerifyingKey::from_bytes(&bytes).context("public key is not a valid Ed25519 point")?;
    Ok((fingerprint(&public), public))
}

/// Verifies a peer by looking its key up in the pin store.
///
/// Used for both directions: a server checking a connecting client, and a
/// client checking the server it dialled. The rule is the same both ways, which
/// is why there is one implementation rather than two that could drift.
#[derive(Debug)]
pub struct PinnedPeerVerifier {
    pins: PinStore,
    /// When set, only this exact peer is acceptable — used when dialling a peer
    /// we intend to reach, so connecting to the wrong node is an error rather
    /// than a silent success.
    expect: Option<NodeId>,
    /// When an unpinned peer is acceptable.
    ///
    /// Only inside the pairing ceremony, where SPAKE2 supplies the
    /// authentication a pin would otherwise. It is a gate rather than a
    /// constant because the agent's own listener has to answer this
    /// differently over time: a stranger is refused at the handshake, except
    /// during the short window in which a human minted a join code.
    allow_unpinned: Unpinned,
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

/// When a peer that is not pinned may still complete a handshake.
#[derive(Clone)]
pub enum Unpinned {
    /// Never. The listener's ordinary posture.
    Never,
    /// Always — the dialling side of a pairing, which has a code in hand.
    Always,
    /// Only while this answers true, i.e. while a join code is outstanding.
    ///
    /// Consulted per handshake, so the window closes the moment the code is
    /// used or expires. Refusing at the handshake rather than after keeps a
    /// stranger's reach limited to rustls' ClientHello handling for all the
    /// time no one is joining, which is almost all of it.
    While(Arc<dyn Fn() -> bool + Send + Sync>),
}

impl std::fmt::Debug for Unpinned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Never => "Never",
            Self::Always => "Always",
            Self::While(_) => "While(..)",
        })
    }
}

impl Unpinned {
    fn allows(&self) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::While(f) => f(),
        }
    }
}

impl PinnedPeerVerifier {
    /// A verifier that accepts only pinned peers.
    #[must_use]
    pub fn pinned(pins: PinStore, expect: Option<NodeId>) -> Arc<Self> {
        Arc::new(Self {
            pins,
            expect,
            allow_unpinned: Unpinned::Never,
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        })
    }

    /// A verifier for the pairing ceremony, where the peer is by definition not
    /// yet pinned and authentication comes from the PAKE instead.
    #[must_use]
    pub fn pairing(pins: PinStore) -> Arc<Self> {
        Arc::new(Self {
            pins,
            expect: None,
            allow_unpinned: Unpinned::Always,
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        })
    }

    /// A verifier that accepts an unpinned peer only while `gate` says so.
    ///
    /// The agent's listener uses this: pinned peers always, a stranger only
    /// inside a join window a human opened.
    #[must_use]
    pub fn while_joining(pins: PinStore, gate: Arc<dyn Fn() -> bool + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            pins,
            expect: None,
            allow_unpinned: Unpinned::While(gate),
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        })
    }

    /// The shared check.
    fn check(&self, end_entity: &CertificateDer<'_>) -> Result<(), rustls::Error> {
        let (id, _key) = peer_identity(end_entity)
            .map_err(|e| rustls::Error::General(format!("peer certificate: {e}")))?;

        if let Some(expected) = self.expect
            && id != expected
        {
            return Err(rustls::Error::General(format!(
                "connected to {id}, expected {expected}"
            )));
        }
        if self.allow_unpinned.allows() {
            return Ok(());
        }
        let pinned = self
            .pins
            .is_pinned(id)
            .map_err(|e| rustls::Error::General(format!("reading the pin store: {e}")))?;
        if pinned {
            Ok(())
        } else {
            Err(rustls::Error::General(format!("peer {id} is not paired")))
        }
    }
}

impl ServerCertVerifier for PinnedPeerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // Server name and expiry are deliberately ignored: identity is the key,
        // and a peer is reached by address, not by a name anyone can vouch for.
        self.check(end_entity)
            .map(|()| ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        // TLS 1.2 is not offered. Reaching here means something negotiated a
        // version we do not accept, so refuse rather than validate.
        Err(rustls::Error::General(
            "TLS 1.2 is not accepted on the peer channel".to_owned(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

impl ClientCertVerifier for PinnedPeerVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.check(end_entity)
            .map(|()| ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(
            "TLS 1.2 is not accepted on the peer channel".to_owned(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

/// Server config for the peer listener.
///
/// # Errors
/// If the certificate cannot be built or rustls rejects the configuration.
pub fn server_config(
    identity: &Identity,
    verifier: Arc<PinnedPeerVerifier>,
) -> Result<rustls::ServerConfig> {
    let pc = certificate_for(identity)?;
    let cfg = rustls::ServerConfig::builder_with_protocol_versions(PROTOCOL_VERSIONS)
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![pc.cert], pc.key)
        .context("configuring the peer listener")?;
    Ok(cfg)
}

/// Client config for dialling a peer.
///
/// # Errors
/// If the certificate cannot be built or rustls rejects the configuration.
pub fn client_config(
    identity: &Identity,
    verifier: Arc<PinnedPeerVerifier>,
) -> Result<rustls::ClientConfig> {
    let pc = certificate_for(identity)?;
    let cfg = rustls::ClientConfig::builder_with_protocol_versions(PROTOCOL_VERSIONS)
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(vec![pc.cert], pc.key)
        .context("configuring the peer dialler")?;
    Ok(cfg)
}
