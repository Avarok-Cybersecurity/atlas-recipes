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
    use atlasctl_agent::pairing::{CODE_TTL_SECS, MAX_ATTEMPTS, PairingCode};
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
            .with_context(|| bind_failure(args.port, local_agent_running()))
    })?;

    println!();
    println!("  Pairing code:  {}", code.grouped());
    println!();
    println!("  On the other machine, run:");
    // The port belongs in the command whenever it is not the one `peer add`
    // assumes. Printing a bare host while listening on 34444 hands the operator
    // a line that dials the wrong port and times out — on the far machine,
    // where they have the least context and no reason to suspect the command
    // they were told to run.
    println!(
        "      atlasctl peer add {} --code {}",
        dial_hints(&dial_hosts(), args.port),
        code.as_str()
    );
    println!();
    println!("  This code is good for {CODE_TTL_SECS} seconds and for {MAX_ATTEMPTS} attempts.");
    println!("  Waiting…");

    let paired = runtime.block_on(async {
        let cfg = server_config(&identity, PinnedPeerVerifier::pairing(pins.clone()))?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(cfg));

        // One connection used to be the entire budget: ANY error below -- a
        // failed TLS handshake, a client that connects and hangs up, or a FORMER
        // peer's five-second poll loop, which is plausible precisely here because
        // this command runs while the local agent is stopped on a machine that
        // may well have been paired before -- ended the command. The operator,
        // still typing `peer add` on the far machine, then had to re-run this and
        // carry a NEW code back across the room.
        //
        // A connection that does not pair is now skipped instead, up to the same
        // MAX_ATTEMPTS the daemon's join window charges. The cap is the point:
        // uncapped, this is an unbounded guessing window for anyone on the LAN,
        // and the TTL alone would not close it.
        let accept = async {
            let mut spent: u8 = 0;
            loop {
                let attempt = async {
                    let (tcp, _) = listener.accept().await?;
                    let mut tls = acceptor
                        .clone()
                        .accept(tcp)
                        .await
                        .context("TLS handshake")?;
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
                }
                .await;

                match attempt {
                    Ok(p) => break Ok(p),
                    Err(e) => {
                        spent += 1;
                        if spent >= MAX_ATTEMPTS {
                            break Err(e).context(format!(
                                "{MAX_ATTEMPTS} connections failed to pair; the code is spent. \
                                 Run `atlasctl agent pair` again for a fresh one."
                            ));
                        }
                        // Named, not swallowed: if a stale peer is eating the
                        // window, its address is the only clue the operator gets.
                        eprintln!(
                            "  a connection did not pair ({e}); still waiting \
                             ({} of {MAX_ATTEMPTS} attempts used)",
                            spent
                        );
                    }
                }
            }
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
        // Pairing authenticates only: the controller grant stays a
        // separate, explicit act (`atlasctl peer grant-control`).
        false,
    )?;
    // Two-phase pairing is two INDEPENDENT decisions, and this side cannot see
    // the other one: a refusal there is a local `return`, and TLS carries the
    // rejection back as a bare alert with none of its reasoning. So a machine
    // that answered "n" — or whose prompt got EOF from a script — leaves this
    // side listing a peer it will never be able to reach, which looks exactly
    // like a box that is switched off. The remedies are unrelated, so say
    // which situation this could be while the operator is still standing at
    // the keyboard.
    // Sanitised: the name is the peer's, and the peer is not trusted yet.
    println!(
        "Paired with {} ({}).",
        atlasctl_protocol::fleet::DisplayName::new(&paired.name).as_str(),
        paired.node.short()
    );
    println!(
        "  {} has to have accepted too. If it said \"Nothing was trusted\",",
        atlasctl_protocol::fleet::DisplayName::new(&paired.name).as_str()
    );
    println!("  pair again — otherwise it will look unreachable from here.");
    Ok(())
}

/// A hostname the other machine can probably reach us on.
///
/// Only ever printed as a hint in a copy-pasteable command — the address that
/// actually matters is whichever one the operator can route to, and they are
/// the ones who know that.
/// Where the other machine should dial this one, best link first.
///
/// The addresses come from the same fabric the agent advertises, NOT from a
/// hostname. `/proc/sys/kernel/hostname` was the old source and it fails two
/// ways: on macOS the path does not exist at all, so a MacBook printed
/// `atlasctl peer add <this-machine>` — a line that cannot be run — and even
/// on Linux a hostname is only dialable if something resolves it, which
/// nothing on a plain LAN promises.
///
/// Falls back to the hostname, then to the placeholder, because a command with
/// a name in it is still a better prompt than no command.
fn dial_hosts() -> Vec<String> {
    use atlasctl_agent::fabric::FabricProvider as _;
    #[cfg(target_os = "macos")]
    let fabric = atlasctl_agent::fabric::macos::MacFabric::new();
    #[cfg(not(target_os = "macos"))]
    let fabric = atlasctl_agent::fabric::linux::LinuxFabric::new();

    let found: Vec<String> = fabric
        .addresses()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.class.usable_for_control())
        .map(|a| a.addr.to_string())
        .collect();
    if found.is_empty() {
        return vec![hostname_hint()];
    }
    found
}

fn hostname_hint() -> String {
    // A placeholder that obviously is not a hostname. This goes into a
    // `peer add` line the operator must edit, and a plausible-looking fallback
    // gets pasted verbatim and dialled.
    atlasctl_core::platform::hostname_or("<this-machine>")
}

/// Every place to dial, as one `peer add` target.
///
/// Comma-separated for the same reason the browser's join command is: this
/// machine cannot know which of its networks the other one shares. A DGX
/// offers its RoCE fabric first — right for another DGX, unreachable from a
/// laptop — and `peer add` walks the list.
#[must_use]
pub fn dial_hints(hosts: &[String], port: u16) -> String {
    hosts
        .iter()
        .map(|h| dial_hint(h, port))
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether this machine's own agent is answering on the browser port.
///
/// Separated from [`bind_failure`] so the message is decided by a pure function
/// a test can drive both ways. A test that probes a real socket passes or fails
/// by accident depending on whether the developer happens to have an agent
/// running — which is exactly what happened when this was one function.
fn local_agent_running() -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], atlasctl_agent::DEFAULT_PORT));
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok()
}

/// Why the peer port could not be bound, in the operator's terms.
///
/// The likely holder is not another `agent pair` — it is this machine's own
/// agent, which binds the same port for the peer channel. That is the COMMON
/// case, because `install.sh` installs the agent as a service: anyone who
/// followed the documented setup and then ran this command was told to go
/// looking for a second copy of a command they had run once.
///
/// A running agent is not a fault here, and the remedy is not to stop it. It
/// takes new members through its join window instead, which is what the control
/// page's "Show me how" opens — so the message names that path rather than
/// sending someone to kill their own agent.
fn bind_failure(port: u16, agent_running: bool) -> String {
    if agent_running {
        format!(
            "could not bind the peer port {port}: this machine's agent is already \
             running and holds it.\n\
             \n\
             That agent adds machines through its own join window, not through this \
             command. On the machine you want to add this one FROM, open the control \
             page and use \"Show me how\" — it hands you one line to run here.\n\
             \n\
             This command is for a machine whose agent is not running yet."
        )
    } else {
        format!(
            "could not bind the peer port {port} — is another `atlasctl agent pair` \
             already waiting?"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::bind_failure;

    /// With no agent listening, the old wording is right: another `agent pair`
    /// really is the likely holder.
    #[test]
    fn with_no_agent_running_it_still_suspects_another_pair() {
        let msg = bind_failure(34334, false);
        assert!(msg.contains("34334"), "{msg}");
        assert!(
            msg.contains("another `atlasctl agent pair`"),
            "without an agent, that is the honest guess: {msg}"
        );
    }

    /// The common case, and the one that was wrong: the agent this operator was
    /// told to install is holding the port, and the old message sent them
    /// hunting for a second copy of a command they ran once.
    #[test]
    fn a_running_agent_is_named_as_the_holder_with_the_path_that_works() {
        let msg = bind_failure(34334, true);
        assert!(
            msg.contains("agent is already") && msg.contains("holds it"),
            "it must name the real holder: {msg}"
        );
        assert!(
            !msg.contains("another `atlasctl agent pair`"),
            "the wrong guess must not survive alongside the right one: {msg}"
        );
        assert!(
            msg.contains("Show me how"),
            "an operator needs the path that DOES work, not just a diagnosis: {msg}"
        );
    }

    /// The message must never send someone to kill the agent they were told to
    /// install.
    #[test]
    fn the_remedy_is_never_to_go_hunting_for_a_process() {
        for running in [true, false] {
            let msg = bind_failure(34334, running);
            assert!(
                !msg.to_lowercase().contains("kill"),
                "an operator following the documented setup must not be told to kill \
                 something: {msg}"
            );
        }
    }
}

/// `host`, or `host:port` when the port is not the one `peer add` assumes.
///
/// Bracketed for an IPv6 literal, because `fe80::1:34444` is not parseable as
/// a host and a port — the colons are ambiguous, and the operator would be the
/// one to discover it.
#[must_use]
pub fn dial_hint(host: &str, port: u16) -> String {
    if port == atlasctl_agent::peer::DEFAULT_PEER_PORT {
        return host.to_owned();
    }
    if host.contains(':') && !host.starts_with('[') {
        return format!("[{host}]:{port}");
    }
    format!("{host}:{port}")
}

#[cfg(test)]
mod dial_tests {
    use super::{dial_hint, dial_hints};
    use atlasctl_agent::peer::DEFAULT_PEER_PORT;

    /// The line has to be runnable on a machine that shares ANY of this one's
    /// networks — a DGX's RoCE fabric for another DGX, its LAN address for a
    /// laptop. `peer add` walks the list, so all of them go in.
    #[test]
    fn every_link_this_machine_has_reaches_the_printed_command() {
        let hosts = vec![
            "10.10.10.9".to_owned(),
            "10.10.10.13".to_owned(),
            "192.168.68.68".to_owned(),
        ];
        assert_eq!(
            dial_hints(&hosts, DEFAULT_PEER_PORT),
            "10.10.10.9,10.10.10.13,192.168.68.68"
        );
    }

    /// The port applies to every entry, not just the first: a partial list is
    /// a line that works until the operator's machine is on the wrong network.
    #[test]
    fn a_non_default_port_is_carried_onto_every_address() {
        let hosts = vec!["10.0.0.1".to_owned(), "fe80::1".to_owned()];
        assert_eq!(dial_hints(&hosts, 34444), "10.0.0.1:34444,[fe80::1]:34444");
    }

    #[test]
    fn one_address_prints_exactly_as_it_did_before() {
        // No comma, no change for the ordinary single-homed machine.
        assert_eq!(
            dial_hints(&["spark-256a".to_owned()], DEFAULT_PEER_PORT),
            "spark-256a"
        );
    }

    #[test]
    fn the_default_port_is_left_off_because_peer_add_assumes_it() {
        assert_eq!(dial_hint("spark-256a", DEFAULT_PEER_PORT), "spark-256a");
    }

    /// The bug: `agent pair --port 34444` printed a bare host, so the line it
    /// told the operator to run dialled 34334 and timed out.
    #[test]
    fn a_non_default_port_is_carried_into_the_command() {
        assert_eq!(dial_hint("spark-256a", 34444), "spark-256a:34444");
        assert_eq!(dial_hint("10.10.10.2", 34444), "10.10.10.2:34444");
    }

    #[test]
    fn an_ipv6_literal_is_bracketed_so_the_port_is_unambiguous() {
        assert_eq!(dial_hint("fe80::1", 34444), "[fe80::1]:34444");
        // Already bracketed input is not double-wrapped.
        assert_eq!(dial_hint("[fe80::1]", 34444), "[fe80::1]:34444");
    }
}
