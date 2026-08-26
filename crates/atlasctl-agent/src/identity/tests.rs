// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

/// A scratch directory that cleans itself up.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "atlasctl-id-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&p).expect("scratch dir");
        Self(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn an_identity_persists_so_a_restart_is_the_same_node() {
    // The whole trust model rests on this: if the id changed on restart, every
    // peer would have to re-pair after a reboot, and people would learn to
    // click through the fingerprint check.
    let tmp = Tmp::new("persist");
    let first = Identity::load_or_create(&tmp.0).expect("first run creates a key");
    let second = Identity::load_or_create(&tmp.0).expect("second run reuses it");
    assert_eq!(first.id(), second.id());
    assert_eq!(first.public().as_bytes(), second.public().as_bytes());
}

#[test]
fn two_agents_are_different_nodes() {
    let a = Identity::generate();
    let b = Identity::generate();
    assert_ne!(a.id(), b.id());
}

#[test]
fn the_id_really_is_the_fingerprint_of_the_public_key() {
    let me = Identity::generate();
    assert_eq!(me.id(), fingerprint(&me.public()));
}

#[cfg(unix)]
#[test]
fn the_private_key_is_not_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = Tmp::new("mode");
    let _ = Identity::load_or_create(&tmp.0).expect("creates");
    let mode = std::fs::metadata(tmp.0.join(KEY_FILE))
        .expect("key exists")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, PRIVATE_MODE, "a private key must be 0600");
}

#[test]
fn a_corrupt_key_file_is_an_error_not_a_new_identity() {
    // Silently regenerating would look like it worked while every existing
    // pairing broke.
    let tmp = Tmp::new("corrupt");
    std::fs::write(tmp.0.join(KEY_FILE), b"not a key").expect("write");
    assert!(Identity::load_or_create(&tmp.0).is_err());
}

fn pin_for(id: NodeId, key: &VerifyingKey) -> Pin {
    Pin {
        id,
        public_key: hex::encode(key.as_bytes()),
        name: DisplayName::new("spark-43fa"),
        paired_at: 1_756_000_000,
    }
}

#[test]
fn a_machine_that_has_never_paired_trusts_nobody() {
    let tmp = Tmp::new("empty");
    let store = PinStore::new(&tmp.0);
    assert!(store.load().expect("empty store reads clean").is_empty());
    assert!(!store.is_pinned(Identity::generate().id()).expect("reads"));
}

#[test]
fn a_pin_survives_a_reload_and_a_removal_takes_effect_at_once() {
    let tmp = Tmp::new("pins");
    let store = PinStore::new(&tmp.0);
    let peer = Identity::generate();
    store
        .add(pin_for(peer.id(), &peer.public()))
        .expect("add pin");

    // Re-reading is what makes revocation immediate: a cached decision would
    // keep a removed peer trusted until restart.
    assert!(PinStore::new(&tmp.0).is_pinned(peer.id()).expect("reads"));
    assert!(store.remove(peer.id()).expect("removes"));
    assert!(!PinStore::new(&tmp.0).is_pinned(peer.id()).expect("reads"));
    assert!(!store.remove(peer.id()).expect("second remove is a no-op"));
}

#[cfg(unix)]
#[test]
fn the_pin_store_is_not_world_readable_either() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = Tmp::new("pinmode");
    let store = PinStore::new(&tmp.0);
    let peer = Identity::generate();
    store.add(pin_for(peer.id(), &peer.public())).expect("add");
    let mode = std::fs::metadata(tmp.0.join(PINS_FILE))
        .expect("exists")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, PRIVATE_MODE);
}

#[test]
fn a_hostile_hostname_in_a_pin_is_sanitised_when_it_is_read_back() {
    // A pin file records a name that arrived over an unauthenticated beacon.
    // It must not become trusted just because it went through a file.
    let tmp = Tmp::new("hostile");
    let store = PinStore::new(&tmp.0);
    let peer = Identity::generate();
    let mut pin = pin_for(peer.id(), &peer.public());
    pin.name = DisplayName::new("evil\u{1b}[31m\u{202e}name");
    store.add(pin).expect("add");

    let back = store.load().expect("reads");
    let name = back[&peer.id()].name.as_str();
    assert!(!name.contains('\u{1b}'));
}

#[test]
fn a_signature_verifies_only_against_the_key_behind_the_pinned_id() {
    let peer = Identity::generate();
    let msg = b"prepare launch qwen3.8-235b-ep2";
    let sig = peer.sign(msg);
    let key_hex = hex::encode(peer.public().as_bytes());

    verify_from(peer.id(), &key_hex, msg, &sig.to_bytes()).expect("the real peer verifies");

    // Right key, wrong message.
    assert!(
        verify_from(
            peer.id(),
            &key_hex,
            b"prepare something else",
            &sig.to_bytes()
        )
        .is_err()
    );

    // A different peer's signature does not pass as this one.
    let impostor = Identity::generate();
    let forged = impostor.sign(msg);
    assert!(verify_from(peer.id(), &key_hex, msg, &forged.to_bytes()).is_err());
}

#[test]
fn claiming_someone_elses_id_while_signing_with_your_own_key_is_refused() {
    // The attack the fingerprint check exists to stop: present a valid
    // signature over a key you really hold, while claiming to be the node the
    // victim pinned. Checking the signature alone would accept this.
    let victim = Identity::generate();
    let attacker = Identity::generate();
    let msg = b"launch";
    let sig = attacker.sign(msg);
    let attacker_key = hex::encode(attacker.public().as_bytes());

    let err = verify_from(victim.id(), &attacker_key, msg, &sig.to_bytes())
        .expect_err("must not authenticate as the victim");
    assert!(
        err.to_string().contains("does not match"),
        "the error must name the fingerprint mismatch, got: {err}"
    );
}

#[test]
fn malformed_key_and_signature_material_is_rejected_rather_than_panicking() {
    let peer = Identity::generate();
    let key_hex = hex::encode(peer.public().as_bytes());
    let sig = peer.sign(b"m");

    assert!(verify_from(peer.id(), "zzzz", b"m", &sig.to_bytes()).is_err());
    assert!(verify_from(peer.id(), "aabb", b"m", &sig.to_bytes()).is_err());
    assert!(verify_from(peer.id(), &key_hex, b"m", b"short").is_err());
}
