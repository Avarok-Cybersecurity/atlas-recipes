// SPDX-License-Identifier: AGPL-3.0-only

//! `recipe list`, `recipe show`, `recipe search`.

use crate::cli::{ListArgs, SearchArgs, ShowArgs};
use crate::hostinfo;
use anyhow::Result;
use atlasctl_core::chain::{Overrides, UserConfig};
use atlasctl_core::docker::collective::NcclRoce;
use atlasctl_core::docker::profile::{NvidiaDevices, ROOTLESS_V1};
use atlasctl_core::docker::translate::{LaunchContext, Placement, translate};
use atlasctl_core::recipe::{Provenance, Recipe};
use atlasctl_core::registry::{RecipeRef, RegistrySet};

/// Load every recipe a registry set knows about.
fn load_all(set: &RegistrySet) -> Vec<Recipe> {
    let mut out = Vec::new();
    for entry in set.list() {
        let r#ref = RecipeRef::Scoped {
            registry: entry.registry,
            name: entry.name,
        };
        if let Ok(r) = set.resolve(&r#ref) {
            out.push(r);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Print the recipe table.
pub fn list(args: &ListArgs) -> Result<()> {
    let set = crate::commands::registry_set()?;
    let recipes = load_all(&set);

    let rows: Vec<&Recipe> = recipes
        .iter()
        .filter(|r| match &args.registry {
            Some(want) => registry_of(r) == *want,
            None => true,
        })
        .filter(|r| args.all || r.is_launchable())
        .collect();

    if rows.is_empty() {
        println!("no recipes matched");
        return Ok(());
    }

    let w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    println!(
        "{:<w$}  {:<8}  {:<6}  MODEL",
        "NAME",
        "REGISTRY",
        "NODES",
        w = w
    );
    for r in &rows {
        let nodes = if r.topology.is_multi_node() {
            format!("{}", r.topology.min_nodes)
        } else {
            "1".to_string()
        };
        println!(
            "{:<w$}  {:<8}  {:<6}  {}",
            r.name,
            registry_of(r),
            nodes,
            r.model,
            w = w
        );
        if let Err(why) = r.launchable() {
            println!("{:<w$}  └─ not launchable: {why}", "", w = w);
        }
    }
    if !args.all {
        let hidden = recipes.len() - rows.len();
        if hidden > 0 {
            println!("\n{hidden} recipe(s) hidden because they cannot be launched; use --all");
        }
    }
    Ok(())
}

/// Print one recipe in detail.
pub fn show(args: &ShowArgs) -> Result<()> {
    let set = crate::commands::registry_set()?;
    let recipe = set.resolve(&RecipeRef::parse(&args.recipe))?;

    if args.docker {
        let host = hostinfo::snapshot()?;
        let ctx = LaunchContext {
            profile: &ROOTLESS_V1,
            devices: &NvidiaDevices,
            collective: &NcclRoce,
        };
        let plan = translate(
            &recipe,
            &Overrides::new(),
            &UserConfig::new(),
            &host,
            &Placement::Solo,
            &ctx,
        )?;
        if args.portable {
            println!(
                "{}",
                plan.docker.display_portable(
                    Some(&host.home),
                    atlasctl_core::platform::home_placeholder()
                )
            );
        } else {
            println!("{}", plan.docker);
        }
        return Ok(());
    }

    println!("name:        {}", recipe.name);
    println!("qualified:   {}", recipe.qualified_name());
    println!("model:       {}", recipe.model);
    println!("image:       {}", recipe.container);
    println!("source:      {}", describe_provenance(&recipe.provenance));
    println!(
        "nodes:       {}{}",
        recipe.topology.min_nodes,
        recipe
            .topology
            .max_nodes
            .map(|m| format!(" (max {m})"))
            .unwrap_or_default()
    );
    match recipe.launchable() {
        Ok(()) => println!("launchable:  yes"),
        Err(why) => println!("launchable:  no — {why}"),
    }
    if let Some(d) = &recipe.description {
        println!("\ndescription:\n{}", indent(d));
    }
    if !recipe.defaults.is_empty() {
        println!("\nsettings:");
        for (k, v) in &recipe.defaults {
            println!("  {k}: {}", v.render());
        }
    }
    if !recipe.unknown_keys.is_empty() {
        println!(
            "\nunrecognised top-level keys (ignored): {}",
            recipe.unknown_keys.join(", ")
        );
    }
    Ok(())
}

/// Search names, models, and descriptions.
pub fn search(args: &SearchArgs) -> Result<()> {
    let set = crate::commands::registry_set()?;
    let needle = args.query.to_lowercase();
    let hits: Vec<Recipe> = load_all(&set)
        .into_iter()
        .filter(|r| {
            r.name.to_lowercase().contains(&needle)
                || r.model.to_lowercase().contains(&needle)
                || r.description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle)
        })
        .collect();

    if hits.is_empty() {
        println!("no recipes matched `{}`", args.query);
        return Ok(());
    }
    for r in hits {
        println!("{}  {}", r.name, r.model);
    }
    Ok(())
}

fn registry_of(r: &Recipe) -> String {
    match &r.provenance {
        Provenance::Builtin { .. } => atlasctl_core::registry::BUILTIN.to_string(),
        Provenance::Remote { registry, .. } => registry.clone(),
        Provenance::LocalPath { .. } => "local".to_string(),
    }
}

fn describe_provenance(p: &Provenance) -> String {
    match p {
        Provenance::Builtin { path } => {
            format!(
                "built in to atlasctl {} ({path})",
                env!("CARGO_PKG_VERSION")
            )
        }
        Provenance::Remote { registry, url } => format!("remote registry `{registry}` ({url})"),
        Provenance::LocalPath { path } => format!("local file {path}"),
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
