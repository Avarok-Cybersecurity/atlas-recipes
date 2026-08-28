// SPDX-License-Identifier: AGPL-3.0-only

//! Command-line surface.

use clap::{Args, Parser, Subcommand};

/// Launch Atlas inference recipes.
#[derive(Parser, Debug)]
#[command(name = "atlasctl", version, about, long_about = None)]
pub struct Cli {
    /// Keep this node's state in a specific directory.
    ///
    /// Relocates `browser.token`, `agent.key` and `peers.json` **together**.
    /// Use this rather than `XDG_CONFIG_HOME`, which moves the identity too
    /// and so returns the node to its own fleet as a stranger.
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<std::path::PathBuf>,

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
    Status(AgentStatusArgs),
    /// Print a code for joining this machine to a fleet.
    ///
    /// The code is shown HERE, on the machine being added, and typed on the
    /// machine doing the adding. That direction is the whole security story: a
    /// web page cannot know a code it did not cause a human to walk over and
    /// read.
    Pair(AgentPairArgs),
    /// Install the agent so it starts on login and survives a reboot.
    ///
    /// A `systemd --user` unit on Linux, a LaunchAgent on macOS. Both run as
    /// you, never as root: the agent already has docker access, and on Linux
    /// that is root-equivalent, so a root-owned service on top would widen a
    /// surface that is wide enough already.
    Install(AgentInstallArgs),
    /// Remove the background service. Leaves the binary and your pairings.
    Uninstall,
}

/// `agent install` arguments.
///
/// These are written into the unit, so they are what the service uses on every
/// future start — not just this one.
#[derive(Args, Debug)]
pub struct AgentInstallArgs {
    /// Port the browser channel should listen on.
    #[arg(long, default_value_t = atlasctl_agent::DEFAULT_PORT)]
    pub port: u16,

    /// Install as a control node: discover, pair and monitor, but never launch.
    ///
    /// For a laptop driving headless machines. Recorded in the unit, so the
    /// machine stays control-only across a reboot rather than quietly becoming
    /// something that runs models.
    #[arg(long)]
    pub client: bool,

    /// Do not advertise on the network, and do not listen for other agents.
    #[arg(long)]
    pub no_discovery: bool,

    /// Do not serve the browser channel, and do not create a pairing token.
    ///
    /// For a machine that exists to hold a rank. Its peers reach it over
    /// mutually authenticated TLS, which never consults the browser token, so
    /// requiring one was a credential it would never use standing between the
    /// node and starting at all.
    #[arg(long)]
    pub no_browser: bool,

    /// Join an existing fleet: `--join <code>@<host>`.
    ///
    /// The whole value is shown by the machine doing the inviting. Installing
    /// and joining are one command because they are one intention, and because
    /// the operator is standing at a machine they may have reached only to run
    /// this.
    #[arg(long, value_name = "CODE@HOST")]
    pub join: Option<String>,

    /// Let the fleet you are joining run models on THIS machine.
    ///
    /// Only meaningful with `--join`. The grant is written into this machine's
    /// pin of the inviter, because this machine's authority is what is being
    /// spent — and it is made here, by whoever is standing at this keyboard
    /// pasting the line, rather than decided remotely by the machine asking.
    ///
    /// A flag rather than a default, and one that appears verbatim in the line
    /// the operator pastes: "consent to remote stop must be said, not implied".
    /// Adding a GPU box in order to drive it is the ordinary reason to be here,
    /// so the invitation offers it — but it says so on the command line, where
    /// it can be read before it is run.
    #[arg(long, requires = "join")]
    pub grant_control: bool,
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
    Remove(PeerNodeArgs),
    /// Let a paired machine drive this one's launch surface, as its own
    /// browser would. Consent to remote stop must be said, not implied by
    /// pairing.
    GrantControl(PeerNodeArgs),
    /// Withdraw that grant. Takes effect on the machine's next connection.
    RevokeControl(PeerNodeArgs),
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

/// Arguments naming one paired machine: `peer remove`, `peer grant-control`,
/// `peer revoke-control`.
#[derive(Args, Debug)]
pub struct PeerNodeArgs {
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

    /// Run as a control node: discover, pair and monitor, but never launch.
    ///
    /// For a laptop driving headless GPU boxes. The refusal is structural — a
    /// client-mode agent reports `can_launch = false` and has no launcher at
    /// all, rather than being trusted to decline.
    #[arg(long)]
    pub client: bool,

    /// Do not advertise on the network, and do not listen for other agents.
    #[arg(long)]
    pub no_discovery: bool,

    /// Do not serve the browser channel, and do not create a pairing token.
    ///
    /// For a machine that exists to hold a rank. Its peers reach it over
    /// mutually authenticated TLS, which never consults the browser token, so
    /// requiring one put a credential the node would never use between it and
    /// starting at all.
    #[arg(long)]
    pub no_browser: bool,

    /// Append this process's output to a file instead of the terminal.
    ///
    /// For a supervised agent whose supervisor captures nothing — which is
    /// every Task Scheduler task. Not a default: a user running `agent run` by
    /// hand wants their output in front of them, and silently diverting it is
    /// how "it printed nothing" becomes a bug report.
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<std::path::PathBuf>,
}

/// `agent status` arguments.
#[derive(Args, Debug)]
pub struct AgentStatusArgs {
    /// Browser port to probe.
    ///
    /// Asked for rather than assumed: `agent install --port 9000` is a
    /// first-class option, and a status check that only ever probed 34333 told
    /// those operators "not running — start it with agent run", which is advice
    /// to start a SECOND agent that then fails to bind against the one that was
    /// running all along.
    #[arg(long, default_value_t = atlasctl_agent::DEFAULT_PORT)]
    pub port: u16,
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own invariants — a malformed definition is a runtime panic on the
    /// user's first `--help`, not a compile error, so it is asserted here.
    #[test]
    fn the_command_definition_is_well_formed() {
        Cli::command().debug_assert();
    }

    /// Granting control to a fleet you are not joining is not a coherent
    /// request. clap should say so rather than accepting it and silently doing
    /// nothing, which is how an operator concludes the flag is broken.
    #[test]
    fn grant_control_without_join_is_refused() {
        let e = Cli::try_parse_from(["atlasctl", "agent", "install", "--grant-control"])
            .expect_err("--grant-control requires --join");
        let msg = e.to_string();
        assert!(
            msg.contains("join"),
            "the error must name what is missing: {msg}"
        );
    }

    /// The pair the installer actually sends.
    #[test]
    fn grant_control_with_join_parses_and_is_off_by_default() {
        let with = Cli::try_parse_from([
            "atlasctl",
            "agent",
            "install",
            "--join",
            "12345678@10.10.10.1",
            "--grant-control",
        ])
        .expect("valid");
        let without = Cli::try_parse_from([
            "atlasctl",
            "agent",
            "install",
            "--join",
            "12345678@10.10.10.1",
        ])
        .expect("valid");

        let grant = |c: Cli| match c.command {
            Command::Agent(AgentCmd::Install(a)) => (a.grant_control, a.join),
            other => panic!("expected agent install, got {other:?}"),
        };
        assert!(grant(with).0);
        // Off unless asked: a privilege must never arrive by upgrading, and the
        // joiner's pin is written from this value.
        assert!(!grant(without).0);
    }
}
