// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::docker::collective::NcclRoce;
use crate::docker::profile::{AmdDevices, NvidiaDevices, ROOTLESS_V1};
use crate::recipe::Provenance;
use crate::scalar::ScalarValue;

pub(super) fn host() -> HostSnapshot {
    HostSnapshot {
        uid: 1000,
        gid: 1000,
        home: "/home/spark".into(),
        hf_cache_dir: "/home/spark/.cache/huggingface".into(),
        // TOKEN stands for a credential the agent holds; the proxy is the
        // one class a recipe may legitimately read.
        env: [
            ("TOKEN".to_string(), "abc".to_string()),
            ("HTTPS_PROXY".to_string(), "http://proxy:8080".to_string()),
        ]
        .into(),
    }
}

pub(super) fn recipe(extra: &str) -> Recipe {
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

pub(super) fn ctx(devices: &dyn super::DeviceProfile) -> LaunchContext<'_> {
    LaunchContext {
        profile: &ROOTLESS_V1,
        devices,
        collective: &NcclRoce,
    }
}

pub(super) fn plan(r: &Recipe, p: &Placement) -> LaunchPlan {
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
    // `swap_space_gb` is a real engine flag this project does not pass through
    // — see `flags::coverage::EXCLUDED`. It still has to be reported rather
    // than dropped, because from the recipe author's side an excluded flag and
    // a misspelt one look identical until someone says so.
    let p = plan(&recipe("  swap_space_gb: 32\n"), &Placement::Solo);
    assert_eq!(p.unmapped.len(), 1);
    assert_eq!(p.unmapped[0].key, "swap_space_gb");
    assert!(
        !p.docker.command.join(" ").contains("swap-space"),
        "an unclaimed setting must not reach the command"
    );
}

#[test]
fn a_claimed_correctness_pin_reaches_the_command() {
    let p = plan(&recipe("  lm_head_dtype: bf16\n"), &Placement::Solo);
    assert!(p.unmapped.is_empty(), "{:?}", p.unmapped);
    assert!(
        p.docker.command.join(" ").contains("--lm-head-dtype bf16"),
        "{:?}",
        p.docker.command
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

/// The whole-pipeline form of the argv-purity property.
///
/// [`crate::settings`] already proves no *setting* can render into something
/// flag-shaped. This proves the stronger thing the security model actually
/// rests on: that after a hostile override map has been validated, translated
/// and rendered, **every `-`-prefixed element of the final docker argv came
/// from a static table in this crate.** Nothing a client sends can become an
/// option, at any layer.
///
/// It runs over the whole flag table rather than a chosen few, so a flag added
/// without thought is caught here rather than in the field.
mod argv_purity {
    use super::*;
    use crate::flags::ATLAS_FLAGS;
    use crate::settings;
    use std::collections::BTreeSet;

    /// Every `-`-prefixed token this crate is allowed to emit.
    ///
    /// Derived from the static tables rather than typed out, so that changing
    /// the profile or adding a flag cannot silently widen what this test
    /// accepts. Some options are emitted joined (`--cap-add=IPC_LOCK`) and some
    /// separated (`--security-opt` then its value), so both forms are built
    /// from the same source.
    fn permitted() -> BTreeSet<String> {
        let mut ok: BTreeSet<String> = ATLAS_FLAGS.iter().map(|f| f.flag.to_string()).collect();
        let p = &ROOTLESS_V1;

        ok.insert(format!("--ipc={}", p.ipc));
        ok.insert(format!("--network={}", p.network));
        ok.insert(format!("--shm-size={}", p.shm_size));
        for c in p.cap_add {
            ok.insert(format!("--cap-add={c}"));
        }
        if p.privileged {
            ok.insert("--privileged".into());
        }

        // Options whose value is a separate argv element, plus the renderer's
        // own fixed tokens.
        for t in [
            "-d",
            "--rm",
            "--name",
            "--entrypoint",
            "--user",
            "-v",
            "-e",
            "--label",
            "--gpus",
            "--device",
            "--security-opt",
            "--ulimit",
            "--restart",
            "--group-add",
        ] {
            ok.insert(t.to_string());
        }
        // Rank placement, emitted by translate itself.
        for t in ["--rank", "--world-size", "--master-addr", "--master-port"] {
            ok.insert(t.to_string());
        }
        ok
    }

    /// Values a hostile client might try, shaped to become options or to break
    /// out of an argument.
    fn hostile() -> Vec<ScalarValue> {
        vec![
            ScalarValue::Str("--privileged".into()),
            ScalarValue::Str("-v /:/host".into()),
            ScalarValue::Str("--entrypoint=/bin/sh".into()),
            ScalarValue::Str("; rm -rf /".into()),
            ScalarValue::Str("$(id -u)".into()),
            ScalarValue::Str("`whoami`".into()),
            ScalarValue::Str("a b --gpus all".into()),
            ScalarValue::Str("--".into()),
            ScalarValue::Str("-".into()),
            ScalarValue::Int(-1),
            ScalarValue::Int(i64::MIN),
            ScalarValue::Float(-1.0),
            ScalarValue::Float(f64::NAN),
            ScalarValue::Float(f64::INFINITY),
            ScalarValue::Bool(true),
        ]
    }

    fn wire(v: &ScalarValue) -> atlasctl_protocol::settings::SettingValue {
        use atlasctl_protocol::settings::SettingValue as S;
        match v {
            ScalarValue::Bool(b) => S::Bool(*b),
            ScalarValue::Int(i) => S::Int(*i),
            ScalarValue::Float(f) => S::Float(*f),
            ScalarValue::Str(s) => S::Str(s.clone()),
        }
    }

    #[test]
    fn no_client_value_can_become_an_option_in_the_final_argv() {
        let allowed = permitted();
        let r = recipe("");
        let mut accepted = 0usize;

        for spec in &ATLAS_FLAGS {
            for v in hostile() {
                let req = [(spec.key.to_string(), wire(&v))].into_iter().collect();
                // Most of these are refused outright, which is the first line of
                // defence. The interesting ones are those that get through.
                let Ok(overrides) = settings::validate(&req) else {
                    continue;
                };
                accepted += 1;

                let plan = translate(
                    &r,
                    &overrides,
                    &UserConfig::new(),
                    &host(),
                    &Placement::Solo,
                    &ctx(&NvidiaDevices),
                )
                .expect("a validated override must still translate");

                for arg in plan.docker.to_argv() {
                    assert!(
                        !arg.starts_with('-') || allowed.contains(&arg),
                        "`{}` = {v:?} produced the option {arg:?}",
                        spec.key
                    );
                }
            }
        }

        // If nothing were ever accepted the loop would prove nothing at all.
        assert!(
            accepted > 0,
            "no hostile value was accepted, so this test asserted nothing"
        );
    }

    /// The rendezvous address is the one value in a rank's command line that
    /// comes from another machine, and it reached argv directly. This test
    /// found that: `master_addr = "--privileged"` emitted `--privileged` as its
    /// own argv element, letting a paired head append serve flags to another
    /// machine's command line.
    #[test]
    fn a_rendezvous_address_that_is_not_an_address_is_refused() {
        let r = recipe("min_nodes: 2\nmax_nodes: 2\n");

        for addr in [
            "--privileged",
            "-v /:/host",
            "10.0.0.1 --gpus all",
            "; rm -rf /",
            "",
            "$(hostname)",
            "head.local",
        ] {
            let out = translate(
                &r,
                &BTreeMap::new(),
                &UserConfig::new(),
                &host(),
                &Placement::Rank {
                    rank: 1,
                    world_size: 2,
                    master_addr: addr.to_string(),
                    master_port: 29500,
                },
                &ctx(&NvidiaDevices),
            );
            assert!(
                matches!(out, Err(TranslateError::BadRendezvousAddress { .. })),
                "master_addr {addr:?} was accepted"
            );
        }
    }

    /// The addresses a fleet actually reports still work, in both families.
    #[test]
    fn a_real_rendezvous_address_still_translates() {
        let allowed = permitted();
        let r = recipe("min_nodes: 2\nmax_nodes: 2\n");

        for addr in ["10.10.10.9", "192.168.1.4", "::1", "fe80::1"] {
            let plan = translate(
                &r,
                &BTreeMap::new(),
                &UserConfig::new(),
                &host(),
                &Placement::Rank {
                    rank: 1,
                    world_size: 2,
                    master_addr: addr.to_string(),
                    master_port: 29500,
                },
                &ctx(&NvidiaDevices),
            )
            .unwrap_or_else(|e| panic!("{addr} must translate: {e}"));

            let argv = plan.docker.to_argv();
            assert!(argv.contains(&addr.to_string()), "{addr} missing from argv");
            for arg in argv {
                assert!(
                    !arg.starts_with('-') || allowed.contains(&arg),
                    "{addr} produced the option {arg:?}"
                );
            }
        }
    }

    /// argv is a vector, never a shell string, so a value containing a
    /// separator stays one inert argument.
    #[test]
    fn a_value_with_shell_syntax_stays_a_single_argument() {
        let req = [(
            "served_model_name".to_string(),
            atlasctl_protocol::settings::SettingValue::Str("a; rm -rf /".into()),
        )]
        .into_iter()
        .collect();

        // It is a denied key, so it never even reaches translate — which is the
        // stronger answer, and the one the schema exists to give.
        assert!(
            settings::validate(&req).is_err(),
            "an unbounded string must not be settable from a client"
        );
    }
}
