// SPDX-License-Identifier: AGPL-3.0-only

//! `atlasctl agent pair` — offering this machine to a fleet, from here.
//!
//! Split from [`super::agent`] for size. This is the original direction of
//! the ceremony, where the code is read off the machine being added. It stays
//! because it is the stronger of the two: nothing has to carry a secret to
//! another machine, so nothing lands in a shell history. The inverted
//! direction in [`atlasctl_agent::joining`] exists for the case this one
//! cannot serve — a headless box with no screen to read a code from.
//!
//! It binds the peer port itself, so it is for a machine whose agent is not
//! already running. A running agent takes new members through its join window.

use crate::hostinfo;
use anyhow::{Context, Result};

/// Print a code for joining this machine to a fleet, and accept one pairing.
///
/// The code is shown HERE and typed on the machine doing the adding. That
/// direction is the entire reason a hostile web page cannot pair anything: it
/// would have to know a code it never saw, on a screen it cannot read.
///
/// # Errors
/// If the identity cannot be loaded or the peer port cannot be bound.
pub fn pair(args: &crate::cli::AgentPairArgs) -> Result<()> {
    use atlasctl_agent::identity::{Identity, PinStore};
    use atlasctl_agent::pairing::{CODE_TTL_SECS, PairingCode};
    use atlasctl_agent::peer::pair::{Role, run};
    use atlasctl_agent::peer::tls::{PinnedPeerVerifier, peer_identity, server_config};
    use atlasctl_protocol::fleet::DisplayName;
    use std::sync::Arc;

    let dir = hostinfo::usable_config_dir()?;
    let identity = Identity::load_or_create(&dir)?;
    let pins = PinStore::new(&dir);
    let code = PairingCode::generate();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;

    // Bind BEFORE printing the code. Printing first and failing after shows
    // someone a code that can never be used — and on a second Spark that
    // already had a pairing waiting, that is exactly what happened.
    let listener = runtime.block_on(async {
        tokio::net::TcpListener::bind(("0.0.0.0", args.port))
            .await
            .with_context(|| {
                format!(
                    "could not bind the peer port {} — is another `atlasctl agent pair` \
                     already waiting?",
                    args.port
                )
            })
    })?;

    println!();
    println!("  Pairing code:  {}", code.grouped());
    println!();
    println!("  On the other machine, run:");
    println!(
        "      atlasctl peer add {} --code {}",
        hostname_hint(),
        code.as_str()
    );
    println!();
    println!("  This code is good for {CODE_TTL_SECS} seconds and for one attempt.");
    println!("  Waiting…");

    let paired = runtime.block_on(async {
        let cfg = server_config(&identity, PinnedPeerVerifier::pairing(pins.clone()))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));

        let accept = async {
            let (tcp, _) = listener.accept().await?;
            let mut tls = acceptor.accept(tcp).await.context("TLS handshake")?;
            let (_, conn) = tls.get_ref();
            let cert = conn
                .peer_certificates()
                .and_then(<[_]>::first)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("the other machine sent no certificate"))?;
            let (peer_id, _) = peer_identity(&cert)?;
            let binding = atlasctl_agent::pairing::binding_from_server(conn)?;
            run(
                &mut tls,
                Role::Responder,
                &identity,
                peer_id,
                code.as_str(),
                binding,
            )
            .await
        };

        // The code expires on its own, so an unattended terminal does not leave
        // a pairing window open indefinitely.
        tokio::time::timeout(std::time::Duration::from_secs(CODE_TTL_SECS), accept)
            .await
            .map_err(|_| anyhow::anyhow!("nobody paired within {CODE_TTL_SECS} seconds"))?
    })?;

    println!();
    println!("  Verification words:  {}", paired.verification);
    println!();
    println!("  The other machine is showing the same words. If it is not,");
    println!("  answer no — something is relaying this connection.");
    println!();
    print!("  Do they match? [y/N] ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("reading confirmation")?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        println!("Nothing was trusted.");
        return Ok(());
    }

    atlasctl_agent::fleet::record_pairing(
        &pins,
        paired.node,
        &paired.public_key,
        DisplayName::new(&paired.name),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        // The responder learns the initiator's address from the connection.
        None,
    )?;
    println!("Paired with {} ({}).", paired.name, paired.node.short());
    Ok(())
}

/// A hostname the other machine can probably reach us on.
///
/// Only ever printed as a hint in a copy-pasteable command — the address that
/// actually matters is whichever one the operator can route to, and they are
/// the ones who know that.
fn hostname_hint() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_else(|_| "<this-machine>".to_owned())
}
