// SPDX-License-Identifier: AGPL-3.0-only

//! `atlasctl peer` — the machines this one trusts.
//!
//! Trust is deliberately hard to acquire and easy to drop. Adding a peer needs
//! a code that only exists on that machine, for two minutes; removing one is a
//! single command that takes effect on the very next connection, because a
//! revocation that needs a restart is not a revocation.

use crate::cli::{PeerAddArgs, PeerNodeArgs};
use anyhow::{Context, Result, bail};
use atlasctl_agent::discovery::resolve_manual;
use atlasctl_agent::identity::{Identity, PinStore};
use atlasctl_agent::peer::DEFAULT_PEER_PORT;
use atlasctl_protocol::fleet::{DisplayName, NodeId};

/// List trusted machines.
///
/// # Errors
/// If the pin store cannot be read.
pub fn list() -> Result<()> {
    let dir = crate::hostinfo::usable_config_dir()?;
    let pins = PinStore::new(&dir).load()?;
    if pins.is_empty() {
        println!("No paired machines.");
        println!();
        // NOT "run `atlasctl agent pair` there". That command binds the peer
        // port, which a running agent already holds — and a machine you would
        // add to a fleet is usually one whose agent is already running, because
        // installing it starts it. Both directions are named, with the
        // condition that picks between them.
        println!("To add one, either:");
        println!("  · open this machine's control page and use \"Show me how\" —");
        println!("    it hands you one line to run on the machine you are adding; or");
        println!("  · if that machine's agent is NOT running, run `atlasctl agent pair`");
        println!("    there and type its code into `atlasctl peer add <host> --code <digits>`.");
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    println!(
        "{:<20}  {:<20}  {:<12}  CONTROL",
        "NAME", "FINGERPRINT", "PAIRED"
    );
    for pin in pins.values() {
        println!(
            "{:<20}  {:<20}  {:<12}  {}",
            pin.name.as_str(),
            pin.id.short(),
            age_text(pin.paired_at, now),
            // The grant that lets that machine drive this one. Invisible until
            // now, which made it impossible to audit: an operator could not
            // answer "who can run commands on my box?" from anywhere.
            if pin.controller { "yes" } else { "—" }
        );
    }
    Ok(())
}

/// How long ago something happened, for a column a human reads.
///
/// `paired_at` is a unix timestamp, and printing it raw put a ten-digit number
/// in front of the operator — technically the answer and useless as one.
///
/// Pure, with `now` passed in, so it is testable without waiting for time to
/// pass. A clock that runs backwards (NTP correction, a pin written on another
/// machine) yields "just now" rather than a negative age.
#[must_use]
pub fn age_text(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=2_591_999 => format!("{} d ago", secs / 86_400),
        _ => format!("{} mo ago", secs / 2_592_000),
    }
}

/// Pair with a machine by address.
///
/// # Errors
/// If the address does not resolve, the machine cannot be reached, or the
/// ceremony fails — which is what both a wrong code and a relayed connection
/// look like.
pub fn add(args: &PeerAddArgs) -> Result<()> {
    let dir = crate::hostinfo::usable_config_dir()?;
    let identity = Identity::load_or_create(&dir)?;
    let pins = PinStore::new(&dir);

    // Every alternative the other machine printed, in the order it offered
    // them. `agent pair` emits a comma-separated target for the same reason
    // the browser's join command carries one: the inviting machine cannot know
    // which of its networks this one shares. A host that will not resolve is
    // recorded rather than fatal — often the LAST entry is the only one this
    // machine's network can even name.
    let mut addrs: Vec<std::net::SocketAddr> = Vec::new();
    let mut unresolved: Vec<String> = Vec::new();
    for host in args
        .target
        .split(',')
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        match resolve_manual(host, DEFAULT_PEER_PORT) {
            Ok(found) => addrs.extend(found),
            Err(e) => unresolved.push(format!("{host}: {e:#}")),
        }
    }
    if addrs.is_empty() {
        anyhow::bail!(
            "{} resolved to no addresses — {}",
            args.target,
            unresolved.join("; ")
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;

    // The same function the browser-driven path calls. Two implementations of
    // a pairing ceremony is one implementation nobody audits.
    //
    // Walked by `peer::reach`, which stops the moment a machine ANSWERS: the
    // addresses are all the same machine, so a refusal has already spent one
    // of the code's attempts and marching through the rest spends them all on
    // one typo.
    let (addr, paired) = atlasctl_agent::peer::reach::walk(&addrs, |addr| {
        runtime.block_on(atlasctl_agent::peer::join::dial_and_pair(
            &identity,
            pins.clone(),
            addr,
            &args.code,
        ))
    })
    .map_err(|e| anyhow::anyhow!("could not pair with {} — {e:#}", args.target))?;

    println!();
    println!("  Verification words:  {}", paired.verification);
    println!();
    println!(
        "  `atlasctl agent pair` on {} is showing the same words.",
        paired.name
    );
    println!("  If it is showing something else, something is relaying this");
    println!("  connection — press Ctrl-C now and nothing will be trusted.");
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
        now_unix(),
        Some(addr.ip().to_string()),
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
    println!("Paired with {} ({}).", paired.name, paired.node.short());
    println!(
        "  {} has to have accepted too. If it said \"Nothing was trusted\",",
        paired.name
    );
    println!("  pair again — otherwise it will look unreachable from here.");
    Ok(())
}

/// Drop trust in a machine.
///
/// # Errors
/// If the prefix matches no peer, or more than one.
pub fn remove(args: &PeerNodeArgs) -> Result<()> {
    let pins = PinStore::new(&crate::hostinfo::usable_config_dir()?);
    let (node, name) = resolve_prefix(&pins, &args.node)?;
    pins.remove(node)?;
    println!("Unpaired {name} ({}).", node.short());
    println!("It will be refused on its next connection.");
    Ok(())
}

/// Let a paired machine drive this one's launch surface.
///
/// # Errors
/// If the prefix matches no peer, or more than one.
pub fn grant_control(args: &PeerNodeArgs) -> Result<()> {
    let pins = PinStore::new(&crate::hostinfo::usable_config_dir()?);
    let (node, name) = resolve_prefix(&pins, &args.node)?;
    // resolve_prefix just proved the pin exists; a false here means it
    // vanished between the read and the write, which must not pass silently.
    anyhow::ensure!(
        pins.set_controller(node, true)?,
        "{name} disappeared from the pin store before the grant was written"
    );
    println!("Granted control to {name} ({}).", node.short());
    println!("It may now start, stop, and inspect launches on this machine,");
    println!("and ask this machine to forward those verbs to its own peers.");
    println!(
        "Withdraw with `atlasctl peer revoke-control {}`.",
        node.short()
    );
    Ok(())
}

/// Withdraw the control grant.
///
/// # Errors
/// If the prefix matches no peer, or more than one.
pub fn revoke_control(args: &PeerNodeArgs) -> Result<()> {
    let pins = PinStore::new(&crate::hostinfo::usable_config_dir()?);
    let (node, name) = resolve_prefix(&pins, &args.node)?;
    anyhow::ensure!(
        pins.set_controller(node, false)?,
        "{name} disappeared from the pin store before the revocation was written"
    );
    println!("Revoked control from {name} ({}).", node.short());
    println!("The machine stays paired; control is refused on its next request.");
    Ok(())
}

/// Resolve a fingerprint prefix to exactly one pinned peer.
///
/// A prefix is what people actually have to hand — the short form printed by
/// `peer list`. Requiring uniqueness rather than taking the first match is
/// what stops an ambiguous prefix from unpairing — or granting control to —
/// the wrong machine.
fn resolve_prefix(pins: &PinStore, prefix: &str) -> Result<(NodeId, String)> {
    let all = pins.load()?;
    let matches: Vec<NodeId> = all
        .keys()
        .filter(|id| id.to_string().starts_with(&prefix.to_lowercase()))
        .copied()
        .collect();

    match matches.as_slice() {
        [] => bail!("no paired machine matches {prefix}"),
        [one] => Ok((*one, all[one].name.as_str().to_owned())),
        many => bail!(
            "{prefix} matches {} machines; use more characters",
            many.len()
        ),
    }
}

/// Seconds since the epoch, or zero if the clock is before it.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod list_tests {
    use super::age_text;

    #[test]
    fn an_age_reads_as_a_human_would_say_it() {
        const H: u64 = 3600;
        const D: u64 = 86_400;
        assert_eq!(age_text(1000, 1000), "just now");
        assert_eq!(age_text(1000, 1059), "just now");
        assert_eq!(age_text(0, 60), "1 min ago");
        assert_eq!(age_text(0, 59 * 60), "59 min ago");
        assert_eq!(age_text(0, H), "1 h ago");
        assert_eq!(age_text(0, 23 * H), "23 h ago");
        assert_eq!(age_text(0, D), "1 d ago");
        assert_eq!(age_text(0, 29 * D), "29 d ago");
        assert_eq!(age_text(0, 40 * D), "1 mo ago");
    }

    /// A pin written on another machine, or an NTP correction, can put
    /// `paired_at` in the future. A negative age would print as a wrapped
    /// number the size of the universe; "just now" is wrong by seconds instead.
    #[test]
    fn a_clock_that_ran_backwards_does_not_wrap() {
        assert_eq!(age_text(9_999, 1_000), "just now");
        assert_eq!(age_text(u64::MAX, 0), "just now");
    }
}
