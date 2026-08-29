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

    // `remove` deliberately leaves the clone on disk, and says so. Re-adding the
    // same registry therefore meets a non-empty destination, where `git clone`
    // fails with "destination path already exists and is not an empty
    // directory" — a message about a directory, for what the operator
    // experienced as re-adding a registry they had just removed.
    match reuse(&dest, &args.url) {
        Reuse::Clone => {
            println!("cloning {} ...", args.url);
            let code = StdProcessRunner.run_streaming(&git_clone_argv(&args.url, &dest))?;
            if code != 0 {
                bail!("`git clone` failed with status {code}; registry not added");
            }
        }
        Reuse::Existing => {
            println!(
                "reusing the clone already at {} — run `atlasctl registry update {}` to refresh it",
                dest.display(),
                args.name
            );
        }
        Reuse::Conflict(found) => {
            bail!(
                "{} already holds a clone of {found}, not {}.\n\
                 Delete that directory or choose another registry name; atlasctl will not \
                 replace a checkout it did not make in this run.",
                dest.display(),
                args.url
            );
        }
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

/// What to do about a destination that may already hold a checkout.
#[derive(Debug, PartialEq, Eq)]
pub enum Reuse {
    /// Nothing there — clone normally.
    Clone,
    /// A checkout of the same URL is already there; keep it.
    Existing,
    /// Something else is there. Named, never overwritten.
    Conflict(String),
}

/// Decide from what is on disk, so the decision is testable without a network.
fn reuse(dest: &std::path::Path, want: &str) -> Reuse {
    let empty = std::fs::read_dir(dest).map(|mut d| d.next().is_none());
    match empty {
        Err(_) => Reuse::Clone,   // does not exist — the ordinary case
        Ok(true) => Reuse::Clone, // exists but empty — git is happy with that
        Ok(false) => {
            let found = StdProcessRunner
                .run(&[
                    "git".into(),
                    "-C".into(),
                    dest.display().to_string(),
                    "remote".into(),
                    "get-url".into(),
                    "origin".into(),
                ])
                .ok()
                .filter(|o| o.success())
                .map(|o| o.stdout.trim().to_owned())
                .unwrap_or_default();
            if same_remote(&found, want) {
                Reuse::Existing
            } else {
                // An unreadable or absent origin is a conflict, not a match:
                // reusing a directory we cannot identify would silently serve
                // recipes from somewhere nobody asked for.
                Reuse::Conflict(if found.is_empty() {
                    "something that is not a git checkout".to_owned()
                } else {
                    found
                })
            }
        }
    }
}

/// Whether two remote URLs name the same repository.
///
/// Compared with a trailing `.git` and any trailing slash ignored, because
/// `…/atlas-recipes` and `…/atlas-recipes.git` are the same place and refusing
/// the second would be a distinction the operator never made.
fn same_remote(a: &str, b: &str) -> bool {
    let norm = |u: &str| {
        u.trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_owned()
    };
    !a.is_empty() && norm(a) == norm(b)
}

#[cfg(test)]
mod reuse_tests {
    use super::{Reuse, reuse, same_remote};

    #[test]
    fn a_url_is_the_same_place_with_or_without_dot_git() {
        let a = "https://github.com/x/atlas-recipes";
        assert!(same_remote(a, "https://github.com/x/atlas-recipes.git"));
        assert!(same_remote("https://github.com/x/atlas-recipes.git/", a));
        assert!(same_remote(a, a));
    }

    #[test]
    fn a_different_repository_is_not_the_same_place() {
        assert!(!same_remote(
            "https://github.com/x/atlas-recipes",
            "https://github.com/y/atlas-recipes"
        ));
        // An unreadable origin must never match: reusing a directory we cannot
        // identify would silently serve recipes from somewhere nobody asked for.
        assert!(!same_remote("", "https://github.com/x/atlas-recipes"));
    }

    #[test]
    fn a_destination_that_does_not_exist_is_an_ordinary_clone() {
        let missing = std::path::Path::new("/definitely/not/here/atlas-registry-test");
        assert_eq!(reuse(missing, "https://example.invalid/r"), Reuse::Clone);
    }

    #[test]
    fn an_empty_destination_is_still_an_ordinary_clone() {
        // git accepts an existing empty directory, so this must not be treated
        // as a conflict — that would refuse a case that works today.
        let dir = std::env::temp_dir().join(format!("atlasctl-reuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        assert_eq!(reuse(&dir, "https://example.invalid/r"), Reuse::Clone);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sequence that motivated this: `remove` leaves the clone on disk and
    /// says so, then re-adding met git's "destination path already exists".
    #[test]
    fn a_non_git_directory_is_a_named_conflict_never_an_overwrite() {
        let dir = std::env::temp_dir().join(format!("atlasctl-reuse-x-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("stray"), b"not a checkout").expect("write");
        match reuse(&dir, "https://example.invalid/r") {
            Reuse::Conflict(found) => assert!(found.contains("not a git checkout"), "{found}"),
            other => panic!("a directory we cannot identify must not be reused: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
