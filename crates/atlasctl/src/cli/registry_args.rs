// SPDX-License-Identifier: AGPL-3.0-only

//! Arguments for `recipe`, `list`, `show`, `search` and `registry`.

use clap::{Args, Subcommand};

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
