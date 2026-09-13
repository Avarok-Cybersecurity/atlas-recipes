// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::cli::{BenchCmd, Cli, Command};
use clap::Parser;

const SHA: &str = "1a0dc88a8c9083bb956bd84cafa2cccbdb8e6e18";

fn spec(params: &[&str], key: Option<&str>) -> JobSpecArgs {
    JobSpecArgs {
        sha: SHA.into(),
        gate: "decode-floor".into(),
        params: params.iter().map(|s| (*s).to_owned()).collect(),
        checkpoint: None,
        hardware: Some("gb10".into()),
        max_run_s: None,
        note: None,
        job_key: key.map(str::to_owned),
    }
}

#[test]
fn a_submit_request_is_validated_here_before_the_node_sees_it() {
    let req = submit_request(&spec(&["osl=8", "conc=2"], None)).unwrap();
    let BenchReq::Submit {
        job_key,
        sha,
        gate,
        params,
        hardware,
        ..
    } = req
    else {
        panic!()
    };
    assert_eq!(sha.as_str(), SHA);
    assert_eq!(gate.as_str(), "decode-floor");
    assert_eq!(params.get("osl").map(String::as_str), Some("8"));
    assert_eq!(hardware.as_deref(), Some("gb10"));
    assert!(
        job_key
            .as_str()
            .starts_with("cli-1a0dc88a8c90-decode-floor-"),
        "{job_key}"
    );

    let mut bad = spec(&[], None);
    bad.sha = "1a0dc88".into();
    assert!(
        submit_request(&bad)
            .unwrap_err()
            .obj
            .message
            .contains("--sha")
    );
    let mut bad = spec(&[], None);
    bad.gate = "-x".into();
    assert!(
        submit_request(&bad)
            .unwrap_err()
            .obj
            .message
            .contains("--gate")
    );
    for p in [&["osl"][..], &["=8"], &["osl=8", "osl=9"]] {
        let e = submit_request(&spec(p, None)).unwrap_err();
        assert_eq!(e.obj.code, "bad_args", "{p:?}");
        assert!(e.obj.message.contains("--param"), "{}", e.obj.message);
    }
    let e = submit_request(&spec(&[], Some("has space"))).unwrap_err();
    assert!(e.obj.message.contains("--job-key"));
}

#[test]
fn the_derived_key_is_a_function_of_sha_gate_and_params_and_always_fits() {
    let sha = Sha::parse(SHA).unwrap();
    let gate = GateId::parse("decode-floor").unwrap();
    let a: BTreeMap<String, String> = [("osl".to_owned(), "8".to_owned())].into();
    let b: BTreeMap<String, String> = [("osl".to_owned(), "9".to_owned())].into();
    assert_eq!(derived_key(&sha, &gate, &a), derived_key(&sha, &gate, &a));
    assert_ne!(derived_key(&sha, &gate, &a), derived_key(&sha, &gate, &b));
    assert_ne!(
        derived_key(&sha, &gate, &a),
        derived_key(&sha, &GateId::parse("ttft-warm-gate").unwrap(), &a)
    );
    // The longest gate id the protocol allows still yields a valid key.
    let long = GateId::parse(&"g".repeat(64)).unwrap();
    let k = derived_key(&sha, &long, &a);
    assert!(k.as_str().len() <= 64, "{}", k.as_str().len());
}

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("atlasctl").chain(args.iter().copied()))
        .unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

#[test]
fn the_command_line_shapes_parse_with_their_defaults() {
    match parse(&[
        "bench",
        "nodes",
        "10.10.10.2,dgx3.local",
        "10.10.10.4:1",
        "--json",
    ])
    .command
    {
        Command::Bench(BenchCmd::Nodes(a)) => {
            assert_eq!(a.nodes, vec!["10.10.10.2", "dgx3.local", "10.10.10.4:1"]);
            assert!(a.out.json);
        }
        other => panic!("{other:?}"),
    }
    match parse(&["bench", "attach", "dgx2", "jb-1-deadbeef"]).command {
        Command::Bench(BenchCmd::Attach(a)) => {
            assert_eq!(a.from_seq, 1);
            assert_eq!(a.reconnect.reconnect_for, 86_400);
            assert!(!a.out.json);
        }
        other => panic!("{other:?}"),
    }
    match parse(&[
        "bench",
        "run",
        "dgx2",
        "--sha",
        SHA,
        "--gate",
        "decode-floor",
        "--param",
        "osl=8",
        "--out-dir",
        "/tmp/x",
        "--reconnect-for",
        "0",
    ])
    .command
    {
        Command::Bench(BenchCmd::Run(a)) => {
            assert_eq!(a.spec.params, vec!["osl=8"]);
            assert_eq!(a.out_dir, std::path::PathBuf::from("/tmp/x"));
            assert_eq!(a.reconnect.reconnect_for, 0);
        }
        other => panic!("{other:?}"),
    }
    // `--out-dir` is required wherever files are written; nothing is
    // guessed about where a record lands.
    assert!(Cli::try_parse_from(["atlasctl", "bench", "fetch", "dgx2", "jb-1-deadbeef"]).is_err());
    assert!(
        Cli::try_parse_from([
            "atlasctl", "bench", "run", "dgx2", "--sha", SHA, "--gate", "g"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["atlasctl", "bench", "submit", "dgx2", "--sha", SHA]).is_err());
}
