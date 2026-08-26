// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! `atlasctl` — launch Atlas inference recipes.

mod cli;
mod commands;
mod hostinfo;
mod peertransport;
mod rankservice;
mod validate;

use anyhow::Result;
use clap::Parser;
use cli::{AgentCmd, Cli, Command, PeerCmd, RecipeCmd, RegistryCmd};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
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
        Command::Agent(AgentCmd::Token(a)) => commands::agent::token(&a),
        Command::Agent(AgentCmd::Status) => commands::agent::status(),
        Command::Agent(AgentCmd::Pair(a)) => commands::agent::pair(&a),
        Command::Peer(PeerCmd::List) => commands::peer::list(),
        Command::Peer(PeerCmd::Add(a)) => commands::peer::add(&a),
        Command::Peer(PeerCmd::Remove(a)) => commands::peer::remove(&a),
        Command::Doctor => commands::doctor::run(),
    }
}
