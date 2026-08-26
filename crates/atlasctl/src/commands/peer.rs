// SPDX-License-Identifier: AGPL-3.0-only

//! `atlasctl peer` — the machines this one trusts.
//!
//! Trust is deliberately hard to acquire and easy to drop. Adding a peer needs
//! a code that only exists on that machine, for two minutes; removing one is a
//! single command that takes effect on the very next connection, because a
//! revocation that needs a restart is not a revocation.

use crate::cli::{PeerAddArgs, PeerRemoveArgs};
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
    let dir = crate::hostinfo::config_dir()?;
    let pins = PinStore::new(&dir).load()?;
    if pins.is_empty() {
        println!("No paired machines.");
        println!();
        println!("Run `atlasctl agent pair` on the machine you want to add, then");
        println!("`atlasctl peer add <host> --code <digits>` here.");
        return Ok(());
    }
    println!("{:<20}  {:<20}  PAIRED", "NAME", "FINGERPRINT");
    for pin in pins.values() {
        println!(
            "{:<20}  {:<20}  {}",
            pin.name.as_str(),
            pin.id.short(),
            pin.paired_at
        );
    }
    Ok(())
}

/// Pair with a machine by address.
///
/// # Errors
/// If the address does not resolve, the machine cannot be reached, or the
/// ceremony fails — which is what both a wrong code and a relayed connection
/// look like.
pub fn add(args: &PeerAddArgs) -> Result<()> {
    let dir = crate::hostinfo::config_dir()?;
    let identity = Identity::load_or_create(&dir)?;
    let pins = PinStore::new(&dir);

    let addrs = resolve_manual(&args.target, DEFAULT_PEER_PORT)?;
    let addr = *addrs
        .first()
        .ok_or_else(|| anyhow::anyhow!("{} resolved to no addresses", args.target))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("starting a runtime")?;

    // The same function the browser-driven path calls. Two implementations of
    // a pairing ceremony is one implementation nobody audits.
    let paired = runtime.block_on(atlasctl_agent::peer::join::dial_and_pair(
        &identity,
        pins.clone(),
        addr,
        &args.code,
    ))?;

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
    )?;
    println!("Paired with {} ({}).", paired.name, paired.node.short());
    Ok(())
}

/// Drop trust in a machine.
///
/// # Errors
/// If the prefix matches no peer, or more than one.
pub fn remove(args: &PeerRemoveArgs) -> Result<()> {
    let dir = crate::hostinfo::config_dir()?;
    let pins = PinStore::new(&dir);
    let all = pins.load()?;

    // A prefix is what people actually have to hand — the short form printed by
    // `peer list`. Requiring uniqueness rather than taking the first match is
    // what stops an ambiguous prefix from unpairing the wrong machine.
    let matches: Vec<NodeId> = all
        .keys()
        .filter(|id| id.to_string().starts_with(&args.node.to_lowercase()))
        .copied()
        .collect();

    match matches.as_slice() {
        [] => bail!("no paired machine matches {}", args.node),
        [one] => {
            let name = all[one].name.as_str().to_owned();
            pins.remove(*one)?;
            println!("Unpaired {name} ({}).", one.short());
            println!("It will be refused on its next connection.");
            Ok(())
        }
        many => {
            bail!(
                "{} matches {} machines; use more characters",
                args.node,
                many.len()
            )
        }
    }
}

/// Seconds since the epoch, or zero if the clock is before it.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
