// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! `atlasctl` — launch Atlas inference recipes.

mod cli;
mod commands;
mod configdir;
mod hostinfo;
mod httpscrape;
mod joinarg;
mod launchtelemetry;
mod peerpairing;
mod peertransport;
mod rankservice;
mod service;
mod validate;

use anyhow::Result;
use clap::Parser;
use cli::{AgentCmd, Cli, Command, PeerCmd, RecipeCmd, RegistryCmd};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // `bench` has a distinct exit code per failure class and, under
            // `--json`, puts the error on stdout where its caller is reading.
            if let Some(b) = e.downcast_ref::<commands::bench::exit::BenchError>() {
                if b.reported {
                    // Already part of the command's own output.
                } else if b.json {
                    match serde_json::to_string(&b.obj) {
                        Ok(line) => println!("{line}"),
                        Err(e) => eprintln!("error: {b}\n  (and could not encode it: {e})"),
                    }
                } else {
                    eprintln!("error: {b}");
                }
                return b.exit.exit();
            }
            // One line per cause, so a nested failure reads as a chain rather
            // than a wall.
            eprintln!("error: {e}");
            for cause in e.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    // Applied before any command runs, so every path that resolves the config
    // directory — including the ones inside the agent library, which never see
    // the CLI — agrees on where state lives. One resolution order, one answer.
    if let Some(dir) = &cli.config_dir {
        // SAFETY: single-threaded here; nothing has been spawned yet.
        unsafe { std::env::set_var(configdir::DIR_ENV, dir) };
    }
    match cli.command {
        Command::Recipe(RecipeCmd::List(a)) | Command::List(a) => commands::recipe::list(&a),
        Command::Recipe(RecipeCmd::Show(a)) | Command::Show(a) => commands::recipe::show(&a),
        Command::Recipe(RecipeCmd::Search(a)) | Command::Search(a) => commands::recipe::search(&a),
        Command::Run(a) => commands::run::run(&a),
        Command::Stop(a) => commands::lifecycle::stop(&a),
        Command::Logs(a) => commands::lifecycle::logs(&a),
        Command::Status => commands::lifecycle::status(),
        Command::Registry(RegistryCmd::List) => commands::registry::list(),
        Command::Registry(RegistryCmd::Add(a)) => commands::registry::add(&a),
        Command::Registry(RegistryCmd::Remove(a)) => commands::registry::remove(&a),
        Command::Registry(RegistryCmd::Update(a)) => commands::registry::update(&a),
        Command::Agent(AgentCmd::Run(a)) => commands::agent::run(&a),
        Command::Agent(AgentCmd::Token(a)) => commands::agentinfo::token(&a),
        Command::Agent(AgentCmd::Status(a)) => commands::agentinfo::status(&a),
        Command::Agent(AgentCmd::Pair(a)) => commands::agentpair::pair(&a),
        Command::Agent(AgentCmd::Install(a)) => commands::service::install(&a),
        Command::Agent(AgentCmd::Uninstall) => commands::service::uninstall(),
        Command::Peer(PeerCmd::List) => commands::peer::list(),
        Command::Peer(PeerCmd::Add(a)) => commands::peer::add(&a),
        Command::Peer(PeerCmd::Remove(a)) => commands::peer::remove(&a),
        Command::Peer(PeerCmd::GrantControl(a)) => commands::peer::grant_control(&a),
        Command::Peer(PeerCmd::RevokeControl(a)) => commands::peer::revoke_control(&a),
        Command::Peer(PeerCmd::GrantBench(a)) => commands::peer::grant_bench(&a),
        Command::Peer(PeerCmd::RevokeBench(a)) => commands::peer::revoke_bench(&a),
        Command::Doctor => commands::doctor::run(),
        Command::Bench(c) => commands::bench::dispatch(&c).map_err(Into::into),
    }
}
