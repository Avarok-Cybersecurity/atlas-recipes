// SPDX-License-Identifier: AGPL-3.0-only

//! Serving a machine that is not pinned yet.
//!
//! Split from [`super`] for size, and because it is a genuinely separate
//! conversation: a stranger inside a join window may pair and nothing else, so
//! every bound and every charge on that path belongs together, here.

use super::peer_of;

/// How long an unpinned caller gets to finish a TLS handshake.
pub(super) const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long an unpinned caller gets to finish the pairing ceremony. Four frames
/// between two machines on a LAN; the generosity is deliberate, the bound is the
/// point.
pub(super) const JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Serve a machine that is not pinned: it may pair, and nothing else.
///
/// Reached only inside a join window, because that is what let it complete a
/// handshake at all. A failure here is ordinary — a mistyped digit, a dropped
/// connection — and is charged against the window's attempt budget rather than
/// logged as an incident.
pub(super) async fn serve_join<S>(
    tls: &mut tokio_rustls::server::TlsStream<S>,
    identity: &crate::identity::Identity,
    pins: &crate::identity::PinStore,
    joining: &crate::joining::JoinWindow,
    fleet: &crate::fleet::LocalFleet,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Charges a guess up front. Every path out of this function below is
    // therefore already accounted for, including the two that return without
    // running a ceremony at all — previously those were free retries.
    //
    // ⚠ The cost of that choice, which is real and not yet paid: this charges
    // for a CONNECTION, not for a guess at the code. Unpair is one-sided by
    // design (`fleet/listing.rs`), so a machine we removed still pins us and its
    // link poller keeps dialing every five seconds. Outside a window those
    // handshakes are refused cheaply — but the moment a human mints a join code,
    // that poller's next three connections complete the handshake, land here,
    // and spend the whole budget in about fifteen seconds. Every "add a machine"
    // afterwards fails with "that invitation was already used", forever, and
    // nothing names the ex-peer as the cause.
    //
    // Fixing it properly means charging only once the caller has actually
    // attempted the PAIRING protocol — a non-pairing first frame is not a guess
    // — which means reading and classifying that frame before `pair::run` owns
    // the stream. That is a change to the ceremony's shape, and it is not made
    // here because getting it subtly wrong turns a rate limit into no rate limit.
    let Some(code) = joining.begin_attempt() else {
        // The window closed between the handshake and here.
        return;
    };
    let Some(peer) = peer_of(tls) else {
        return;
    };
    let binding = {
        let (_, conn) = tls.get_ref();
        match crate::pairing::binding_from_server(conn) {
            Ok(b) => b,
            Err(_) => return,
        }
    };

    // The failure arm is deliberately empty: `begin_attempt` already charged
    // this guess, so there is nothing left to record when the ceremony fails.
    if let Ok(paired) = crate::peer::pair::run(
        tls,
        crate::peer::pair::Role::Responder,
        identity,
        peer,
        &code,
        binding,
    )
    .await
    {
        // Single use: the invitation is spent whether or not the pin
        // write below succeeds, because the code has now been seen on the
        // wire by whoever answered.
        //
        // Losing this race means another ceremony already spent the
        // invitation. Both peers hold a valid code, so neither is
        // necessarily hostile — but "one invitation, one machine" is the
        // property, and admitting the loser would quietly break it.
        let Some(consumed) = joining.consume() else {
            eprintln!(
                "refusing {}: that invitation was already used by another machine. \
                     Mint a fresh one to add this node.",
                paired.node.short()
            );
            return;
        };
        if let Err(e) = crate::fleet::record_pairing(
            pins,
            paired.node,
            &paired.public_key,
            atlasctl_protocol::fleet::DisplayName::new(&paired.name),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            None,
            // The grant the human chose when they minted this invitation,
            // written with the pin so consent and trust land atomically.
            consumed.allow_control,
        ) {
            // Announcing a pairing whose pin never reached disk is how a
            // fleet ends up with one side believing it is paired and the
            // other rejecting it on the next connection, with nothing
            // anywhere saying why.
            eprintln!(
                "pairing with {} completed but could not be recorded: {e:#}. \
                     The peer believes it is paired; this machine does not.",
                paired.node.short()
            );
            return;
        }
        let _ = fleet;
        // The words, not just the name. The machine that DIALLED shows these to
        // its operator and asks them to compare — and until now this side
        // printed nothing to compare against, so the comparison was one-sided
        // and the question the other dialog asked could not be answered.
        //
        // This is a log line rather than a prompt because this side is
        // typically headless and unattended: the ceremony is authorised by the
        // invitation, which a human minted here minutes ago. The words let
        // someone who wants to check, check.
        eprintln!(
            "paired with {} ({}) — verification words: {}",
            // Sanitised: the name is the joining peer's own claim, and this
            // line goes into the journal, where a control sequence is just as
            // effective at rewriting what a reader sees.
            atlasctl_protocol::fleet::DisplayName::new(&paired.name).as_str(),
            paired.node.short(),
            paired.verification
        );
    }
}
