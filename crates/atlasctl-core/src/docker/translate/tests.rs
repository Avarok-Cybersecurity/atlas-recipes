// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::docker::collective::NcclRoce;
use crate::docker::profile::{AmdDevices, NvidiaDevices, ROOTLESS_V1};
use crate::recipe::Provenance;
use crate::scalar::ScalarValue;

fn host() -> HostSnapshot {
    HostSnapshot {
        uid: 1000,
        gid: 1000,
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
        env: [("TOKEN".to_string(), "abc".to_string())].into(),
    }
}

fn recipe(extra: &str) -> Recipe {
    let yaml = format!(
        "model: org/m\ncontainer: img:tag\nruntime: atlas\n\
         defaults:\n  port: 8888\n  gpu_memory_utilization: 0.88\n{extra}"
    );
    Recipe::parse(
        "r",
        &yaml,
        Provenance::Builtin {
            path: "r.yaml".into(),
        },
    )
    .expect("parses")
}

fn ctx(devices: &dyn super::DeviceProfile) -> LaunchContext<'_> {
    LaunchContext {
        profile: &ROOTLESS_V1,
        devices,
        collective: &NcclRoce,
    }
}

fn plan(r: &Recipe, p: &Placement) -> LaunchPlan {
    translate(
        r,
        &Overrides::new(),
        &UserConfig::new(),
        &host(),
        p,
        &ctx(&NvidiaDevices),
    )
    .expect("translates")
}

#[test]
fn a_solo_launch_serves_the_model_with_its_resolved_flags() {
    let p = plan(&recipe(""), &Placement::Solo);
    assert_eq!(
        p.docker.command,
        [
            "spark",
            "serve",
            "org/m",
            "--port",
            "8888",
            "--gpu-memory-utilization",
            "0.88"
        ]
    );
    assert_eq!(p.docker.name, "atlas-r");
    assert_eq!(p.docker.image, "img:tag");
}

#[test]
fn translation_is_byte_stable() {
    let r = recipe("");
    assert_eq!(
        plan(&r, &Placement::Solo).docker.to_string(),
        plan(&r, &Placement::Solo).docker.to_string(),
        "the website prints this string; it must not vary"
    );
}

#[test]
fn the_standard_environment_is_offline_by_default() {
    let p = plan(&recipe(""), &Placement::Solo);
    assert_eq!(p.docker.env["HF_HUB_OFFLINE"], "1");
    assert_eq!(p.docker.env["TRANSFORMERS_OFFLINE"], "1");
    assert_eq!(p.docker.env["HF_HOME"], "/cache/huggingface");
}

#[test]
fn recipe_env_overrides_the_standard_block_and_expands_variables() {
    let p = plan(
        &recipe("env:\n  HF_HOME: /custom\n  KEY: \"pre-$TOKEN\"\n"),
        &Placement::Solo,
    );
    assert_eq!(
        p.docker.env["HF_HOME"], "/custom",
        "the recipe is the more specific intent"
    );
    assert_eq!(p.docker.env["KEY"], "pre-abc");
}

#[test]
fn the_model_cache_is_mounted_from_the_host() {
    let p = plan(&recipe(""), &Placement::Solo);
    assert_eq!(
        p.docker.volumes["/home/spark/.cache/huggingface"],
        "/cache/huggingface"
    );
}

#[test]
fn containers_are_labelled_so_they_survive_an_agent_restart() {
    let p = plan(&recipe(""), &Placement::Solo);
    assert!(
        p.docker
            .labels
            .contains(&(LABEL_MANAGED.into(), "1".into()))
    );
    assert!(p.docker.labels.contains(&(LABEL_RECIPE.into(), "r".into())));
}

#[test]
fn the_head_rank_serves_the_api_and_carries_coordination_flags() {
    let r = recipe("min_nodes: 2\nmax_nodes: 2\n");
    let p = plan(
        &r,
        &Placement::Rank {
            rank: 0,
            world_size: 2,
            master_addr: "10.10.10.1".into(),
            master_port: DEFAULT_MASTER_PORT,
        },
    );
    let c = p.docker.command.join(" ");
    assert!(
        c.contains("--port 8888"),
        "the head keeps its resolved port: {c}"
    );
    assert!(c.contains("--rank 0 --world-size 2 --master-addr 10.10.10.1 --master-port 29500"));
    assert_eq!(p.docker.name, "atlas-r-rank0");
}

#[test]
fn a_worker_rank_is_forced_to_port_zero() {
    let r = recipe("min_nodes: 2\nmax_nodes: 2\n");
    let p = plan(
        &r,
        &Placement::Rank {
            rank: 1,
            world_size: 2,
            master_addr: "10.10.10.1".into(),
            master_port: DEFAULT_MASTER_PORT,
        },
    );
    let c = p.docker.command.join(" ");
    assert!(c.contains("--port 0"), "a worker serves no API: {c}");
    assert!(
        !c.contains("--port 8888"),
        "the resolved port must not also appear: {c}"
    );
    assert_eq!(p.docker.name, "atlas-r-rank1");
}

#[test]
fn a_multi_node_recipe_refuses_to_launch_on_one_node() {
    // The failure mode this guards: quietly serving a different model than the
    // recipe describes.
    let r = recipe("min_nodes: 2\nmax_nodes: 2\n");
    let err = translate(
        &r,
        &Overrides::new(),
        &UserConfig::new(),
        &host(),
        &Placement::Solo,
        &ctx(&NvidiaDevices),
    )
    .expect_err("must refuse");
    assert!(matches!(
        err,
        TranslateError::NodeCountMismatch {
            required: 2,
            supplied: 1,
            ..
        }
    ));
}

#[test]
fn a_recipe_carrying_executable_content_never_translates() {
    let r = recipe("mods:\n  - some-mod\n");
    let err = translate(
        &r,
        &Overrides::new(),
        &UserConfig::new(),
        &host(),
        &Placement::Solo,
        &ctx(&NvidiaDevices),
    )
    .expect_err("must refuse");
    assert!(matches!(err, TranslateError::NotLaunchable { .. }));
}

#[test]
fn overrides_flow_through_the_chain_into_the_command() {
    let p = translate(
        &recipe(""),
        &Overrides::from([("port".to_string(), ScalarValue::Int(9999))]),
        &UserConfig::new(),
        &host(),
        &Placement::Solo,
        &ctx(&NvidiaDevices),
    )
    .expect("translates");
    assert!(p.docker.command.join(" ").contains("--port 9999"));
}

#[test]
fn unmapped_recipe_settings_are_reported_on_the_plan() {
    let p = plan(&recipe("  lm_head_dtype: bf16\n"), &Placement::Solo);
    assert_eq!(p.unmapped.len(), 1);
    assert_eq!(p.unmapped[0].key, "lm_head_dtype");
    assert!(
        !p.docker.command.join(" ").contains("lm_head"),
        "an unclaimed setting must not reach the command"
    );
}

#[test]
fn the_same_recipe_translates_on_a_different_vendor_without_special_casing() {
    // The agnosticism guarantee: only the device flags differ.
    let r = recipe("");
    let nvidia = plan(&r, &Placement::Solo);
    let amd = translate(
        &r,
        &Overrides::new(),
        &UserConfig::new(),
        &host(),
        &Placement::Solo,
        &ctx(&AmdDevices),
    )
    .expect("translates");

    assert_eq!(
        nvidia.docker.command, amd.docker.command,
        "the serve command is vendor-neutral"
    );
    assert_ne!(nvidia.docker.device_flags, amd.docker.device_flags);
    assert!(amd.docker.to_string().contains("/dev/kfd"));
    assert!(!amd.docker.to_string().contains("--gpus"));
}
