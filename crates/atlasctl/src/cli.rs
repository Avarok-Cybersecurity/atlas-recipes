// SPDX-License-Identifier: AGPL-3.0-only

//! Command-line surface.

use clap::{Args, Parser, Subcommand};

/// Launch Atlas inference recipes.
#[derive(Parser, Debug)]
#[command(name = "atlasctl", version, about, long_about = None)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands.
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Find and inspect recipes.
    #[command(subcommand)]
    Recipe(RecipeCmd),

    /// List available recipes.
    List(ListArgs),

    /// Show one recipe in detail.
    Show(ShowArgs),

    /// Search recipes by name, model, or description.
    Search(SearchArgs),

    /// Launch a recipe.
    Run(RunArgs),

    /// Stop a running recipe.
    Stop(StopArgs),

    /// Follow a running recipe's logs.
    Logs(LogsArgs),

    /// Show what is currently running.
    Status,

    /// Manage recipe registries.
    #[command(subcommand)]
    Registry(RegistryCmd),

    /// Run and manage the local agent that the website talks to.
    #[command(subcommand)]
    Agent(AgentCmd),

    /// Manage the machines this one trusts.
    #[command(subcommand)]
    Peer(PeerCmd),

    /// Check this machine for problems, including a compromised sparkrun install.
    Doctor,
}

/// Recipe subcommands.
#[derive(Subcommand, Debug)]
pub enum RecipeCmd {
    /// List available recipes.
    List(ListArgs),
    /// Show one recipe in detail.
    Show(ShowArgs),
    /// Search recipes.
    Search(SearchArgs),
}

/// Agent subcommands.
#[derive(Subcommand, Debug)]
pub enum AgentCmd {
    /// Run the agent in the foreground.
    Run(AgentRunArgs),
    /// Print the pairing token to paste into the website.
    Token(AgentTokenArgs),
    /// Report whether an agent is reachable.
    Status,
    /// Print a code for joining this machine to a fleet.
    ///
    /// The code is shown HERE, on the machine being added, and typed on the
    /// machine doing the adding. That direction is the whole security story: a
    /// web page cannot know a code it did not cause a human to walk over and
    /// read.
    Pair(AgentPairArgs),
}

/// `agent pair` arguments.
#[derive(Args, Debug)]
pub struct AgentPairArgs {
    /// Port the peer channel listens on.
    #[arg(long, default_value_t = atlasctl_agent::peer::DEFAULT_PEER_PORT)]
    pub port: u16,
}

/// Peer subcommands.
///
/// `peer add` is a first-class path, not a fallback for when discovery fails.
/// Enterprise wireless does client isolation, plenty of switches filter
/// multicast, and the RoCE links between two Sparks are point-to-point /30s
/// where multicast reaches exactly one machine anyway.
#[derive(Subcommand, Debug)]
pub enum PeerCmd {
    /// List machines this one trusts, and those it can see.
    List,
    /// Pair with a machine by address, using a code read off that machine.
    Add(PeerAddArgs),
    /// Drop trust in a machine. Takes effect on its next connection.
    Remove(PeerRemoveArgs),
}

/// `peer add` arguments.
#[derive(Args, Debug)]
pub struct PeerAddArgs {
    /// Host or host:port of the machine to pair with.
    pub target: String,

    /// The eight digits shown by `atlasctl agent pair` on that machine.
    #[arg(long)]
    pub code: String,
}

/// `peer remove` arguments.
#[derive(Args, Debug)]
pub struct PeerRemoveArgs {
    /// Fingerprint, or a unique prefix of one.
    pub node: String,
}

/// `agent run` arguments.
#[derive(Args, Debug)]
pub struct AgentRunArgs {
    /// Port to listen on.
    #[arg(long, default_value_t = atlasctl_agent::DEFAULT_PORT)]
    pub port: u16,

    /// Also accept connections from a local development server.
    #[arg(long)]
    pub dev_origins: bool,
}

/// `agent token` arguments.
#[derive(Args, Debug)]
pub struct AgentTokenArgs {
    /// Replace the token, invalidating any browser already paired.
    #[arg(long)]
    pub rotate: bool,
}

/// Registry subcommands.
#[derive(Subcommand, Debug)]
pub enum RegistryCmd {
    /// List configured registries.
    List,
    /// Add a registry. Added registries supply recipe data only; they can never
    /// cause a command to run.
    Add(RegistryAddArgs),
    /// Remove a registry.
    Remove(RegistryRemoveArgs),
    /// Update registries from git.
    Update(RegistryUpdateArgs),
}

/// `recipe list` arguments.
#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only show recipes from this registry.
    #[arg(long, value_name = "NAME")]
    pub registry: Option<String>,

    /// Include recipes that cannot be launched, with the reason.
    #[arg(long)]
    pub all: bool,
}

/// `recipe show` arguments.
#[derive(Args, Debug)]
pub struct ShowArgs {
    /// Recipe reference: `name` or `@registry/name`.
    pub recipe: String,

    /// Print the `docker run` command this recipe implies and exit.
    #[arg(long)]
    pub docker: bool,

    /// With `--docker`, keep host specifics symbolic so the command can be
    /// pasted on another machine.
    #[arg(long)]
    pub portable: bool,
}

/// `recipe search` arguments.
#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Text to look for.
    pub query: String,
}

/// `run` arguments.
#[derive(Args, Debug)]
pub struct RunArgs {
    /// Recipe reference: `name` or `@registry/name`.
    pub recipe: String,

    /// Override a recipe setting, e.g. `-o max_model_len=8192`.
    #[arg(short = 'o', long = "option", value_name = "KEY=VALUE")]
    pub options: Vec<String>,

    /// Port the model server listens on.
    #[arg(long)]
    pub port: Option<u16>,

    /// Use a different container image than the recipe names.
    #[arg(long, value_name = "IMAGE")]
    pub image: Option<String>,

    /// Print the command instead of running it.
    #[arg(long)]
    pub print: bool,

    /// With `--print`, keep host specifics symbolic.
    #[arg(long)]
    pub portable: bool,

    /// Keep the container after it exits, so its logs survive a crash.
    #[arg(long)]
    pub no_rm: bool,

    /// Skip pulling the image.
    #[arg(long)]
    pub no_pull: bool,

    /// This node's rank in a multi-node launch.
    #[arg(long, requires_all = ["world_size", "master_addr"])]
    pub rank: Option<u16>,

    /// Total nodes in a multi-node launch.
    #[arg(long)]
    pub world_size: Option<u16>,

    /// Address all ranks rendezvous on.
    #[arg(long, value_name = "ADDR")]
    pub master_addr: Option<String>,

    /// Port all ranks rendezvous on.
    #[arg(long, default_value_t = atlasctl_core::docker::translate::DEFAULT_MASTER_PORT)]
    pub master_port: u16,
}

/// `stop` arguments.
#[derive(Args, Debug)]
pub struct StopArgs {
    /// Recipe name, or omit with `--all`.
    pub recipe: Option<String>,

    /// Stop every recipe atlasctl started.
    #[arg(long)]
    pub all: bool,
}

/// `logs` arguments.
#[derive(Args, Debug)]
pub struct LogsArgs {
    /// Recipe name.
    pub recipe: String,

    /// Follow the log stream.
    #[arg(short, long)]
    pub follow: bool,

    /// Lines of history to show first.
    #[arg(long, default_value_t = 100)]
    pub tail: u32,
}

/// `registry add` arguments.
#[derive(Args, Debug)]
pub struct RegistryAddArgs {
    /// Local name for the registry.
    pub name: String,
    /// Git URL to clone.
    pub url: String,
    /// Subdirectory within the repository that holds recipes.
    #[arg(long, default_value = "recipes")]
    pub subpath: String,
}

/// `registry remove` arguments.
#[derive(Args, Debug)]
pub struct RegistryRemoveArgs {
    /// Registry to remove.
    pub name: String,
}

/// `registry update` arguments.
#[derive(Args, Debug)]
pub struct RegistryUpdateArgs {
    /// Update only this registry.
    pub name: Option<String>,
}
