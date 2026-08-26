// SPDX-License-Identifier: AGPL-3.0-only

//! The pairing ceremony: turning "I can see that machine" into "I trust it".
//!
//! A short numeric code is all a human will actually transcribe, and a short
//! code sent over a channel is trivially guessable offline. SPAKE2 fixes that:
//! it turns a low-entropy shared secret into a strong shared key, and gives an
//! attacker exactly **one online guess per attempt** rather than an offline
//! dictionary. Wrong guesses are therefore rate-limitable, and they are
//! rate-limited.
//!
//! **The code always originates on the target machine.** Someone runs
//! `atlasctl agent pair` on the box they want to add, and reads the digits off
//! that screen. This is what makes a hostile web page harmless: it cannot know
//! a code it did not cause a human to walk over and read.
//!
//! **The TLS exporter is mixed into the transcript.** This is the part that
//! actually defeats a machine-in-the-middle. An attacker who terminates two
//! separate TLS sessions and relays between them ends up with two *different*
//! exporter secrets, so the confirmation MACs do not match on either side, key
//! confirmation fails, and no pin is written. Without that binding, SPAKE2
//! would authenticate the endpoints while an attacker sat happily between two
//! authenticated connections.

use anyhow::{Context, Result, bail};
use atlasctl_protocol::fleet::NodeId;
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

#[cfg(test)]
mod tests;

/// How many digits a pairing code carries.
///
/// Eight digits is 10^8 possibilities. SPAKE2 allows one online guess per
/// exchange, and the lockout below caps attempts, so the realistic attack is
/// three guesses in a minute rather than a dictionary run.
pub const CODE_DIGITS: usize = 8;

/// How long a code is valid for.
pub const CODE_TTL_SECS: u64 = 120;

/// Wrong attempts allowed before the lockout.
pub const MAX_ATTEMPTS: u8 = 3;

/// How long a locked-out peer must wait.
pub const LOCKOUT_SECS: u64 = 60;

/// The TLS exporter label used for channel binding.
const EXPORTER_LABEL: &[u8] = b"atlasctl pairing channel binding v1";

/// Bytes of exporter material mixed into the transcript.
const EXPORTER_LEN: usize = 32;

/// A freshly generated pairing code.
///
/// Zeroised on drop, and never logged: it is a secret for the two minutes it
/// lives.
pub struct PairingCode(String);

impl std::fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairingCode(********)")
    }
}

impl Drop for PairingCode {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl PairingCode {
    /// Generate a code from OS entropy.
    ///
    /// Rejection-sampled rather than reduced modulo 10: taking `byte % 10`
    /// would make the digits 0-5 slightly likelier than 6-9, and biased digits
    /// shrink the search space of the one guess an attacker gets.
    ///
    /// # Panics
    /// If the OS cannot supply entropy.
    #[must_use]
    pub fn generate() -> Self {
        let mut digits = String::with_capacity(CODE_DIGITS);
        let mut buf = [0u8; 1];
        while digits.len() < CODE_DIGITS {
            getrandom::fill(&mut buf).expect("the OS must supply entropy");
            // 250 is the largest multiple of 10 below 256; anything above it is
            // discarded so every digit is equally likely.
            if buf[0] < 250 {
                digits.push(char::from(b'0' + buf[0] % 10));
            }
        }
        Self(digits)
    }

    /// The digits, for display on the machine that generated them.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Grouped for reading aloud: `1234 5678`.
    #[must_use]
    pub fn grouped(&self) -> String {
        let (a, b) = self.0.split_at(CODE_DIGITS / 2);
        format!("{a} {b}")
    }
}

/// Whether a string could be a pairing code.
///
/// Shape only. Whether it is the *right* code is decided by the exchange, never
/// by a comparison here.
#[must_use]
pub fn looks_like_code(s: &str) -> bool {
    s.len() == CODE_DIGITS && s.bytes().all(|b| b.is_ascii_digit())
}

/// One side's in-progress exchange.
///
/// Split into start/finish so the caller owns the message transport and this
/// module owns only the cryptography.
pub struct Exchange {
    state: Spake2<Ed25519Group>,
    /// Exporter material from the TLS session this exchange runs inside.
    binding: Vec<u8>,
    /// Who we are.
    local: NodeId,
    /// Who we believe we are talking to.
    remote: NodeId,
}

/// The outbound message plus the state needed to finish.
pub struct Started {
    /// The SPAKE2 message to send to the peer.
    pub message: Vec<u8>,
    /// Keep this to finish the exchange.
    pub exchange: Exchange,
}

impl Exchange {
    /// Begin an exchange.
    ///
    /// `binding` is the TLS exporter material for the connection this runs
    /// over; both sides must derive it with the same label and length, and the
    /// values will differ if anything is relaying between them.
    ///
    /// The two node ids are mixed in as SPAKE2 identities so a message captured
    /// from one pairing cannot be replayed into another.
    #[must_use]
    pub fn start(code: &str, local: NodeId, remote: NodeId, binding: Vec<u8>) -> Started {
        // Symmetric SPAKE2: neither agent is "the server", and both derive the
        // same key. The identities are ordered so both sides agree regardless
        // of who dialled.
        let (lo, hi) = if local <= remote {
            (local, remote)
        } else {
            (remote, local)
        };
        let (state, message) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(code.as_bytes()),
            &SpakeIdentity::new(format!("{lo}:{hi}").as_bytes()),
        );
        Started {
            message,
            exchange: Self {
                state,
                binding,
                local,
                remote,
            },
        }
    }

    /// Complete the exchange with the peer's message, producing a confirmation
    /// MAC to exchange and the key both sides should agree on.
    ///
    /// # Errors
    /// If the peer's message is malformed. A *wrong code* does not fail here —
    /// it produces a different key, which is caught by the confirmation step.
    /// That distinction matters: failing early on a wrong code would leak
    /// whether the code was right before confirmation, which is the offline
    /// oracle SPAKE2 exists to remove.
    pub fn finish(self, peer_message: &[u8]) -> Result<Confirmation> {
        let key = self
            .state
            .finish(peer_message)
            .map_err(|e| anyhow::anyhow!("pairing message was malformed: {e:?}"))?;
        Ok(Confirmation {
            key,
            binding: self.binding,
            local: self.local,
            remote: self.remote,
        })
    }
}

/// The key-confirmation half of the ceremony.
pub struct Confirmation {
    key: Vec<u8>,
    binding: Vec<u8>,
    local: NodeId,
    remote: NodeId,
}

impl std::fmt::Debug for Confirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Confirmation")
            .field("remote", &self.remote)
            .finish_non_exhaustive()
    }
}

impl Drop for Confirmation {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl Confirmation {
    /// The MAC this side sends.
    #[must_use]
    pub fn mine(&self) -> [u8; 32] {
        self.mac(self.local, self.remote)
    }

    /// The MAC this side expects to receive.
    #[must_use]
    pub fn theirs(&self) -> [u8; 32] {
        self.mac(self.remote, self.local)
    }

    /// Check the peer's MAC in constant time.
    ///
    /// # Errors
    /// If it does not match — which is what a wrong code, or a relay sitting
    /// between two TLS sessions, both look like.
    pub fn verify(&self, received: &[u8]) -> Result<()> {
        let expected = self.theirs();
        if received.len() == expected.len() && bool::from(expected.ct_eq(received)) {
            Ok(())
        } else {
            bail!("key confirmation failed: wrong code, or something is relaying this connection")
        }
    }

    /// A short verification string both humans can compare, derived from the
    /// agreed key rather than from either fingerprint alone.
    #[must_use]
    pub fn verification_words(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"atlasctl verification v1");
        h.update(&self.key);
        let d = h.finalize();
        format!("{:02x}{:02x}-{:02x}{:02x}", d[0], d[1], d[2], d[3])
    }

    fn mac(&self, from: NodeId, to: NodeId) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"atlasctl key confirmation v1");
        h.update(&self.key);
        // The channel binding is what makes a relayed pairing fail: two TLS
        // sessions produce two different exporters.
        h.update(
            u32::try_from(self.binding.len())
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        h.update(&self.binding);
        h.update(from.as_bytes());
        h.update(to.as_bytes());
        h.finalize().into()
    }
}

/// Export channel-binding material from a client connection.
///
/// # Errors
/// If rustls cannot export keying material.
pub fn binding_from_client(conn: &rustls::ClientConnection) -> Result<Vec<u8>> {
    let mut out = vec![0u8; EXPORTER_LEN];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, None)
        .context("exporting TLS keying material")?;
    Ok(out)
}

/// Export channel-binding material from a server connection.
///
/// # Errors
/// If rustls cannot export keying material.
pub fn binding_from_server(conn: &rustls::ServerConnection) -> Result<Vec<u8>> {
    let mut out = vec![0u8; EXPORTER_LEN];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, None)
        .context("exporting TLS keying material")?;
    Ok(out)
}
