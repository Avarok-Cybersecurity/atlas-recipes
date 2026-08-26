// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn node(b: u8) -> NodeId {
    NodeId::from_bytes([b; 32])
}

/// Run both halves of a ceremony and report whether confirmation succeeded.
fn ceremony(
    code_a: &str,
    code_b: &str,
    binding_a: &[u8],
    binding_b: &[u8],
) -> Result<(String, String)> {
    let (a, b) = (node(0xa1), node(0xb2));
    let sa = Exchange::start(code_a, a, b, binding_a.to_vec());
    let sb = Exchange::start(code_b, b, a, binding_b.to_vec());

    let ca = sa.exchange.finish(&sb.message)?;
    let cb = sb.exchange.finish(&sa.message)?;

    ca.verify(&cb.mine())?;
    cb.verify(&ca.mine())?;
    Ok((ca.verification_words(), cb.verification_words()))
}

#[test]
fn a_code_is_eight_unbiased_digits() {
    let c = PairingCode::generate();
    assert_eq!(c.as_str().len(), CODE_DIGITS);
    assert!(looks_like_code(c.as_str()));
    assert_eq!(c.grouped().len(), CODE_DIGITS + 1);

    // Rejection sampling, not `% 10`: over many draws every digit should appear.
    // A modulo bias would starve 6-9 relative to 0-5.
    let mut seen = [0u32; 10];
    for _ in 0..400 {
        for d in PairingCode::generate().as_str().bytes() {
            seen[usize::from(d - b'0')] += 1;
        }
    }
    let lo = *seen.iter().min().expect("ten buckets");
    let hi = *seen.iter().max().expect("ten buckets");
    assert!(lo > 0, "every digit must be reachable");
    // 3200 draws over 10 buckets: a modulo bias would show ~20% skew. Allow
    // generous sampling noise but catch a systematic lean.
    assert!(
        f64::from(hi) < f64::from(lo) * 1.6,
        "digit distribution looks biased: {seen:?}"
    );
}

#[test]
fn a_code_never_appears_in_a_debug_line() {
    // It is a secret for two minutes, and secrets that can be printed are.
    let c = PairingCode::generate();
    let printed = format!("{c:?}");
    assert!(!printed.contains(c.as_str()), "the code leaked into Debug");
    assert!(printed.contains("****"));
}

#[test]
fn shape_checking_a_code_never_decides_whether_it_is_correct() {
    assert!(looks_like_code("12345678"));
    assert!(!looks_like_code("1234567"));
    assert!(!looks_like_code("123456789"));
    assert!(!looks_like_code("1234567a"));
    assert!(!looks_like_code(""));
}

#[test]
fn the_same_code_over_the_same_channel_agrees_and_confirms() {
    let binding = b"tls-exporter-material-shared".to_vec();
    let (wa, wb) = ceremony("13572468", "13572468", &binding, &binding)
        .expect("a genuine pairing must complete");
    assert_eq!(wa, wb, "both humans must see the same verification words");
    assert_eq!(wa.len(), 9, "short enough to read across a desk");
}

#[test]
fn a_wrong_code_fails_at_confirmation_not_earlier() {
    // Failing at `finish` would tell an attacker the code was wrong before
    // confirmation — the offline oracle SPAKE2 exists to remove. It must get
    // all the way to a MAC comparison and then fail.
    let binding = b"same-channel".to_vec();
    let err = ceremony("13572468", "13572469", &binding, &binding)
        .expect_err("a wrong code must not pair");
    assert!(
        err.to_string().contains("key confirmation failed"),
        "expected a confirmation failure, got: {err}"
    );
}

#[test]
fn a_relayed_pairing_fails_even_with_the_correct_code() {
    // THE test. An attacker terminates two separate TLS sessions and relays
    // SPAKE2 messages between them. Both sides run the right code, so SPAKE2
    // alone would succeed — but each side's TLS exporter belongs to its own
    // session, so the confirmation MACs are computed over different bindings
    // and neither side accepts the other.
    let victim_binding = b"exporter-of-session-one".to_vec();
    let attacker_binding = b"exporter-of-session-two".to_vec();
    let err = ceremony("13572468", "13572468", &victim_binding, &attacker_binding)
        .expect_err("a machine-in-the-middle must not be able to complete a pairing");
    assert!(err.to_string().contains("relaying") || err.to_string().contains("confirmation"));
}

#[test]
fn a_message_from_one_pairing_cannot_be_replayed_into_another() {
    // The node ids are mixed in as SPAKE2 identities, so a message captured
    // between A and B is not a valid message between A and C.
    let binding = b"channel".to_vec();
    let (a, b, c) = (node(1), node(2), node(3));

    let a_to_b = Exchange::start("11112222", a, b, binding.clone());
    let c_side = Exchange::start("11112222", c, a, binding.clone());

    // Feed C's message into an exchange that expected B's.
    let conf = a_to_b.exchange.finish(&c_side.message).expect("parses");
    let c_conf = c_side.exchange.finish(&a_to_b.message).expect("parses");
    assert!(
        conf.verify(&c_conf.mine()).is_err(),
        "identities must bind the exchange to this specific pair of nodes"
    );
}

#[test]
fn both_sides_agree_regardless_of_who_dialled() {
    // The exchange is symmetric and orders identities, so pairing works the
    // same whether A dialled B or B dialled A.
    let binding = b"c".to_vec();
    let (a, b) = (node(0x10), node(0x20));

    let s1 = Exchange::start("55556666", a, b, binding.clone());
    let s2 = Exchange::start("55556666", b, a, binding.clone());
    let c1 = s1.exchange.finish(&s2.message).expect("finishes");
    let c2 = s2.exchange.finish(&s1.message).expect("finishes");
    c1.verify(&c2.mine()).expect("A accepts B");
    c2.verify(&c1.mine()).expect("B accepts A");
}

#[test]
fn a_malformed_peer_message_is_an_error_rather_than_a_panic() {
    let s = Exchange::start("12341234", node(1), node(2), b"c".to_vec());
    assert!(s.exchange.finish(b"garbage").is_err());
}

#[test]
fn a_code_is_worth_guessing_only_online() {
    // The security argument is that SPAKE2 gives one guess per exchange, so the
    // code space only has to beat the attempt limit — not an offline
    // dictionary. Assert the space is actually as large as the digit count
    // claims, by generating codes and checking they span more than a handful of
    // values (a constant-folded generator would collapse this).
    let mut codes = std::collections::BTreeSet::new();
    for _ in 0..200 {
        codes.insert(PairingCode::generate().as_str().to_owned());
    }
    assert!(
        codes.len() > 190,
        "codes must not repeat: {} distinct out of 200",
        codes.len()
    );
}
