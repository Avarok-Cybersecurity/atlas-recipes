// SPDX-License-Identifier: AGPL-3.0-only

//! This agent's cryptographic identity, and the peers it has decided to trust.
//!
//! Identity is an Ed25519 keypair generated once on first run and kept at
//! `0600`. Its [`NodeId`] is the SHA-256 of the public key. That indirection is
//! what lets the peer channel pin a *key* rather than a certificate: certs
//! expire and get regenerated, and a pairing that broke every time a cert
//! rolled over would train people to re-pair without checking, which is the
//! whole ceremony defeated.
//!
//! Trust is a pin file: a peer this machine has completed the pairing ceremony
//! with. Discovery does not write here. Only pairing does, and only after key
//! confirmation. Removing a pin takes effect on the next connection, because
//! every connection re-reads the store rather than caching a decision made at
//! startup.

use anyhow::{Context, Result};
use atlasctl_protocol::fleet::{DisplayName, NodeId};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

#[cfg(test)]
mod tests;

/// Filename of the private key inside the agent's config directory.
const KEY_FILE: &str = "agent.key";
/// Filename of the pin store.
const PINS_FILE: &str = "peers.json";

/// The fingerprint of a public key.
#[must_use]
pub fn fingerprint(public: &VerifyingKey) -> NodeId {
    let mut h = Sha256::new();
    h.update(public.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    NodeId::from_bytes(digest)
}

/// This agent's keypair.
///
/// The signing key is never serialised anywhere but its own `0600` file, and
/// never logged: [`std::fmt::Debug`] is implemented by hand to print the
/// fingerprint and nothing else, because a key that can appear in a log line
/// eventually does.
pub struct Identity {
    signing: SigningKey,
    id: NodeId,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("id", &self.id).finish()
    }
}

impl Identity {
    /// Generate a fresh identity from OS entropy.
    ///
    /// Seeded straight from `getrandom` rather than through an RNG trait: the
    /// only randomness this needs is 32 bytes from the kernel, and going
    /// directly makes that auditable in one line instead of through whichever
    /// `rand_core` version the dependency graph settled on.
    ///
    /// # Panics
    /// If the operating system cannot supply entropy, which is not a condition
    /// this program can sensibly continue past.
    #[must_use]
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("the OS must be able to supply 32 bytes of entropy");
        let signing = SigningKey::from_bytes(&seed);
        seed.zeroize();
        let id = fingerprint(&signing.verifying_key());
        Self { signing, id }
    }

    /// Load the identity at `dir`, generating and persisting one if absent.
    ///
    /// # Errors
    /// If the directory cannot be created, or the key file exists but cannot be
    /// read or is not a valid key.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating agent directory {}", dir.display()))?;
        let path = dir.join(KEY_FILE);
        if path.exists() {
            let raw =
                std::fs::read(&path).with_context(|| format!("reading key {}", path.display()))?;
            let bytes: [u8; 32] = raw
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", path.display()))?;
            let signing = SigningKey::from_bytes(&bytes);
            let id = fingerprint(&signing.verifying_key());
            return Ok(Self { signing, id });
        }
        let me = Self::generate();
        atlasctl_core::secretfile::write(&path, me.signing.as_bytes())?;
        Ok(me)
    }

    /// This node's id.
    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// This node's public key.
    #[must_use]
    pub fn public(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// Sign a message.
    #[must_use]
    pub fn sign(&self, msg: &[u8]) -> Signature {
        self.signing.sign(msg)
    }

    /// Borrow the signing key, for building a TLS certificate.
    #[must_use]
    pub const fn signing_key(&self) -> &SigningKey {
        &self.signing
    }
}

/// One peer this machine has paired with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    /// The peer's identity.
    pub id: NodeId,
    /// Its public key, hex encoded.
    pub public_key: String,
    /// Hostname at pairing time. Display only, and re-sanitised on read.
    pub name: DisplayName,
    /// When it was paired, as a unix timestamp.
    pub paired_at: u64,
    /// Where it was last seen.
    ///
    /// Remembered so that restarting this agent does not blank a paired
    /// machine's address until mDNS happens to re-announce it — which can take
    /// a minute on a quiet network and looks exactly like the peer having no
    /// usable link. Defaulted so pin files written before this field still
    /// load.
    #[serde(default)]
    pub last_address: Option<String>,
    /// Whether this peer may CONTROL this machine: execute the closed
    /// `ControlReq` vocabulary here (terminal `Control` frame), and ask this
    /// machine to forward that vocabulary one hop to its own pins
    /// (`ControlTo` frame). One bit checked at both accept points, so the
    /// grant means the same thing wherever it is consulted.
    ///
    /// Defaults to false so every pin written before this field — and every
    /// pairing ceremony ever performed — keeps exactly the authority it had:
    /// the six container-scoped rank verbs and `serve_query`. Without this
    /// default, upgrading the fleet would retroactively turn every existing
    /// pin into a license to stop the operator's local workloads.
    #[serde(default)]
    pub controller: bool,
}

/// The set of peers this machine trusts.
///
/// Deliberately a plain file re-read on every connection rather than a cached
/// map: `atlasctl peer remove` has to take effect immediately, and a revocation
/// that needs a restart is not a revocation.
#[derive(Debug, Clone, Default)]
pub struct PinStore {
    path: PathBuf,
}

impl PinStore {
    /// A store backed by `dir/peers.json`.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(PINS_FILE),
        }
    }

    /// Every pinned peer, by id.
    ///
    /// A missing file is an empty set, not an error: a machine that has never
    /// paired is the normal case.
    ///
    /// # Errors
    /// If the file exists but cannot be read or parsed.
    pub fn load(&self) -> Result<BTreeMap<NodeId, Pin>> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let raw = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let pins: Vec<Pin> = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", self.path.display()))?;
        Ok(pins.into_iter().map(|p| (p.id, p)).collect())
    }

    /// Whether a peer is trusted.
    ///
    /// # Errors
    /// If the store cannot be read.
    pub fn is_pinned(&self, id: NodeId) -> Result<bool> {
        Ok(self.load()?.contains_key(&id))
    }

    /// Add a pin, replacing any existing one for the same id.
    ///
    /// # Errors
    /// If the store cannot be read or written.
    pub fn add(&self, pin: Pin) -> Result<()> {
        let mut pins = self.load()?;
        pins.insert(pin.id, pin);
        self.save(&pins)
    }

    /// Remove a pin. Returns whether it was there.
    ///
    /// # Errors
    /// If the store cannot be read or written.
    pub fn remove(&self, id: NodeId) -> Result<bool> {
        let mut pins = self.load()?;
        let had = pins.remove(&id).is_some();
        if had {
            self.save(&pins)?;
        }
        Ok(had)
    }

    /// Grant or revoke the `controller` right on an existing pin. Returns
    /// whether the pin existed.
    ///
    /// Re-reads the file and writes it back whole, like [`Self::add`] and
    /// [`Self::remove`], so a revocation takes effect on the very next
    /// connection — the same property the pin store itself exists for.
    ///
    /// # Errors
    /// If the store cannot be read or written.
    pub fn set_controller(&self, id: NodeId, granted: bool) -> Result<bool> {
        let mut pins = self.load()?;
        let Some(pin) = pins.get_mut(&id) else {
            return Ok(false);
        };
        pin.controller = granted;
        self.save(&pins)?;
        Ok(true)
    }

    fn save(&self, pins: &BTreeMap<NodeId, Pin>) -> Result<()> {
        let list: Vec<&Pin> = pins.values().collect();
        let json = serde_json::to_string_pretty(&list).context("serialising pins")?;
        atlasctl_core::secretfile::write(&self.path, json.as_bytes())
    }
}

/// Verify a signature against a claimed public key, and check that the key
/// really is the one behind an id.
///
/// Both halves matter. Checking only the signature lets a peer sign with a key
/// that is not the one you pinned; checking only the fingerprint lets it claim
/// a key it does not hold.
///
/// # Errors
/// If the key is malformed, the fingerprint does not match, or the signature
/// does not verify.
pub fn verify_from(id: NodeId, public_key_hex: &str, msg: &[u8], sig: &[u8]) -> Result<()> {
    let key_bytes: [u8; 32] = hex::decode(public_key_hex)
        .context("public key is not hex")?
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key is not 32 bytes"))?;
    let public = VerifyingKey::from_bytes(&key_bytes).context("public key is not on the curve")?;
    anyhow::ensure!(
        fingerprint(&public) == id,
        "public key does not match the pinned fingerprint"
    );
    let sig_bytes: [u8; 64] = sig
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature is not 64 bytes"))?;
    public
        .verify(msg, &Signature::from_bytes(&sig_bytes))
        .context("signature did not verify")
}

/// Check that a hex public key really is the key behind an id.
///
/// Used where a peer offers a key to be pinned: without this it could
/// authenticate with one key and ask to be recorded under another, which would
/// pin an identity nobody ever proved.
///
/// # Errors
/// If the key is malformed or does not hash to `id`.
pub fn verify_key_matches(id: NodeId, public_key_hex: &str) -> Result<()> {
    let key_bytes: [u8; 32] = hex::decode(public_key_hex)
        .context("public key is not hex")?
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key is not 32 bytes"))?;
    let public = VerifyingKey::from_bytes(&key_bytes).context("public key is not on the curve")?;
    anyhow::ensure!(
        fingerprint(&public) == id,
        "public key does not match the fingerprint it claims"
    );
    Ok(())
}
