// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn sample() -> DockerCommand {
    DockerCommand {
        detach: true,
        entrypoint: Some(String::new()),
        privileged: false,
        device_flags: vec!["--gpus".into(), "all".into()],
        ipc: "host".into(),
        shm_size: "32gb".into(),
        network: "host".into(),
        user: Some(UserSpec {
            uid: 1000,
            gid: 1000,
        }),
        security_opts: vec!["no-new-privileges".into()],
        cap_add: vec!["IPC_LOCK".into()],
        ulimits: vec!["memlock=-1:-1".into()],
        devices: vec!["/dev/infiniband".into()],
        memory: None,
        labels: vec![("io.atlasctl.recipe".into(), "r".into())],
        auto_remove: true,
        restart: None,
        name: "atlas-r".into(),
        env: [("HF_HOME".to_string(), "/cache/huggingface".to_string())].into(),
        volumes: [(
            "/home/spark/.cache/huggingface".to_string(),
            "/cache/huggingface".to_string(),
        )]
        .into(),
        image: "avarok/atlas-gb10:latest".into(),
        command: vec![
            "spark".into(),
            "serve".into(),
            "org/m".into(),
            "--port".into(),
            "8888".into(),
        ],
    }
}

#[test]
fn argv_starts_with_docker_run_and_ends_with_the_serve_command() {
    let argv = sample().to_argv();
    assert_eq!(&argv[..2], ["docker", "run"]);
    assert_eq!(
        &argv[argv.len() - 5..],
        ["spark", "serve", "org/m", "--port", "8888"]
    );
}

#[test]
fn the_image_immediately_precedes_the_container_command() {
    let argv = sample().to_argv();
    let img = argv
        .iter()
        .position(|a| a == "avarok/atlas-gb10:latest")
        .expect("image present");
    assert_eq!(
        argv[img + 1],
        "spark",
        "the command must follow the image directly"
    );
}

#[test]
fn the_cleared_entrypoint_survives_as_an_empty_argument() {
    let argv = sample().to_argv();
    let i = argv
        .iter()
        .position(|a| a == "--entrypoint")
        .expect("entrypoint present");
    assert_eq!(
        argv[i + 1],
        "",
        "an empty entrypoint clears the image's own"
    );
    assert!(sample().to_string().contains("--entrypoint ''"));
}

#[test]
fn no_shell_metacharacter_can_reach_argv_unquoted() {
    // Execution is argv-only, so a hostile value is inert; prove it stays one
    // element rather than becoming several.
    let mut c = sample();
    c.command.push("; rm -rf /".into());
    let argv = c.to_argv();
    assert_eq!(
        argv.last().unwrap(),
        "; rm -rf /",
        "must remain a single argument"
    );
    assert!(
        c.to_string().contains(r"'; rm -rf /'"),
        "and must be quoted for display"
    );
}

#[test]
fn env_and_volumes_render_in_sorted_order() {
    let mut c = sample();
    c.env.insert("ZZZ".into(), "last".into());
    c.env.insert("AAA".into(), "first".into());
    let argv = c.to_argv();
    let envs: Vec<String> = argv
        .windows(2)
        .filter(|w| w[0] == "-e")
        .map(|w| w[1].clone())
        .collect();
    assert_eq!(
        envs.first().unwrap(),
        "AAA=first",
        "env must be sorted for stable output"
    );
    assert_eq!(envs.last().unwrap(), "ZZZ=last");
}

#[test]
fn rendering_is_deterministic_across_calls() {
    let c = sample();
    assert_eq!(c.to_argv(), c.to_argv());
    assert_eq!(c.to_string(), c.to_string());
}

#[test]
fn the_portable_rendering_keeps_host_specifics_symbolic() {
    let portable = sample().display_portable(Some("/home/spark"));
    assert!(
        portable.contains("$(id -u):$(id -g)"),
        "uid must stay symbolic"
    );
    assert!(
        portable.contains("$HOME/.cache/huggingface"),
        "home must stay symbolic"
    );
    assert!(
        !portable.contains("/home/spark"),
        "no literal home should remain"
    );
    assert!(sample().to_string().contains("--user 1000:1000"));
}

#[test]
fn a_user_spec_brings_the_mounts_that_make_it_usable() {
    let argv = sample().to_argv();
    assert!(
        argv.windows(2)
            .any(|w| w[0] == "-v" && w[1] == "/etc/passwd:/etc/passwd:ro")
    );
    assert!(
        argv.windows(2)
            .any(|w| w[0] == "-v" && w[1] == "/etc/group:/etc/group:ro")
    );
}

#[test]
fn omitted_options_emit_nothing() {
    let mut c = sample();
    c.user = None;
    c.auto_remove = false;
    c.detach = false;
    c.entrypoint = None;
    let line = c.to_string();
    for absent in ["--user", "--rm", "--entrypoint", "--privileged", "--memory"] {
        assert!(!line.contains(absent), "{absent} should not appear: {line}");
    }
}
