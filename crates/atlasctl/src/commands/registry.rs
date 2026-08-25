// SPDX-License-Identifier: AGPL-3.0-only

//! `registry list|add|remove|update`.

use crate::cli::{RegistryAddArgs, RegistryRemoveArgs, RegistryUpdateArgs};
use crate::hostinfo;
use anyhow::{Result, bail};
use atlasctl_core::io::{ProcessRunner, StdFileSystem, StdProcessRunner};
use atlasctl_core::registry::{
    BUILTIN, RemoteRegistry, RemoteStore, git_clone_argv, git_update_argv,
};

fn store() -> Result<(RemoteStore, std::path::PathBuf)> {
    let path = crate::commands::registries_path()?;
    Ok((RemoteStore::load(&StdFileSystem, &path)?, path))
}

/// Show configured registries.
pub fn list() -> Result<()> {
    let (store, _) = store()?;
    println!("{:<12}  {:<8}  SOURCE", "NAME", "KIND");
    println!(
        "{:<12}  {:<8}  compiled into atlasctl {}",
        BUILTIN,
        "builtin",
        env!("CARGO_PKG_VERSION")
    );
    for r in &store.registries {
        println!("{:<12}  {:<8}  {}", r.name, "remote", r.url);
    }
    if store.registries.is_empty() {
        println!(
            "\nNo remote registries configured. Recipes ship inside atlasctl, so a fresh\n\
             install needs no network access at all."
        );
    } else {
        println!(
            "\nRemote registries supply recipe data only. They cannot cause a command to run:\n\
             recipes carrying executable-content keys are refused wherever they come from."
        );
    }
    Ok(())
}

/// Add a registry and clone it.
pub fn add(args: &RegistryAddArgs) -> Result<()> {
    let (mut store, path) = store()?;
    let dest = hostinfo::cache_dir()?.join("registries").join(&args.name);

    let registry: RemoteRegistry = serde_yaml_ng::from_str(&format!(
        "name: {}\nurl: {}\nsubpath: {}\npath: {}\n",
        args.name,
        args.url,
        args.subpath,
        dest.display()
    ))?;
    // Validate the name before touching the network.
    store.add(registry)?;

    println!("cloning {} ...", args.url);
    let code = StdProcessRunner.run_streaming(&git_clone_argv(&args.url, &dest))?;
    if code != 0 {
        bail!("`git clone` failed with status {code}; registry not added");
    }

    store.save(&StdFileSystem, &path)?;
    println!("added `{}`", args.name);
    println!(
        "note: recipes from `{}` are never trusted. atlasctl will refuse any that carry\n\
         executable-content keys, exactly as it does for built-in recipes.",
        args.name
    );
    Ok(())
}

/// Remove a registry from the configuration.
///
/// The clone is left on disk. Deleting a directory tree on the user's behalf is
/// not something a `remove` from a config list should imply.
pub fn remove(args: &RegistryRemoveArgs) -> Result<()> {
    let (mut store, path) = store()?;
    if !store.remove(&args.name) {
        bail!("no registry named `{}`", args.name);
    }
    store.save(&StdFileSystem, &path)?;
    println!("removed `{}`", args.name);
    println!(
        "its clone is still on disk at {}; delete it yourself if you want the space back",
        hostinfo::cache_dir()?
            .join("registries")
            .join(&args.name)
            .display()
    );
    Ok(())
}

/// Update registry clones from git.
pub fn update(args: &RegistryUpdateArgs) -> Result<()> {
    let (store, _) = store()?;
    let cache_root = hostinfo::cache_dir()?.join("registries");

    let targets: Vec<&RemoteRegistry> = match &args.name {
        Some(name) => {
            let Some(r) = store.registries.iter().find(|r| &r.name == name) else {
                bail!("no registry named `{name}`");
            };
            vec![r]
        }
        None => store.registries.iter().collect(),
    };

    if targets.is_empty() {
        println!("no remote registries configured; nothing to update");
        return Ok(());
    }

    for r in targets {
        // The update is a hard reset, so confirm the target is ours first.
        RemoteStore::guard_cache_path(&cache_root, &r.path)?;
        print!("updating {} ... ", r.name);
        for argv in git_update_argv(&r.path) {
            let out = StdProcessRunner.run(&argv)?;
            if !out.success() {
                println!("failed");
                bail!("`{}` failed: {}", argv.join(" "), out.stderr.trim());
            }
        }
        println!("done");
    }
    Ok(())
}
