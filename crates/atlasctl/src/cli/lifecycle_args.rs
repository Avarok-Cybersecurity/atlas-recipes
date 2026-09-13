// SPDX-License-Identifier: AGPL-3.0-only

//! Arguments for `run`, `stop` and `logs`.

use clap::Args;

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
