// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_protocol::msg::bench_event::ArtifactKind;

fn meta(name: &str, rel: &str, data: &[u8]) -> ArtifactMeta {
    ArtifactMeta {
        name: name.into(),
        relative_path: rel.into(),
        bytes: data.len() as u64,
        sha256: hex::encode(Sha256::digest(data)),
        kind: ArtifactKind::Record,
    }
}

#[test]
fn an_artifact_lands_at_its_relative_path_under_the_out_dir() {
    let out = Path::new("/tmp/certify");
    let m = meta(
        "2026-09-13-1a0dc88a8c.json",
        ".benchmarks/decode-floor/2026-09-13-1a0dc88a8c.json",
        b"{}",
    );
    assert_eq!(
        destination(out, &m).unwrap(),
        PathBuf::from("/tmp/certify/.benchmarks/decode-floor/2026-09-13-1a0dc88a8c.json")
    );
}

/// The node's manifest names the child log `child.log` and places it as
/// `.certify/<sha>/<gate>.log` — the two fields are a key and a place, not one
/// string twice. Campaign attempt 2 of stack #1073 lost a finished
/// `bfcl-subset-echolp-b` shard to a CLI that insisted they match.
#[test]
fn an_artifact_may_land_under_a_different_name_than_its_manifest_key() {
    let out = Path::new("/tmp/certify");
    let m = meta(
        "child.log",
        ".certify/732ead61db/bfcl-subset-echolp-b.log",
        b"log",
    );
    assert_eq!(
        destination(out, &m).unwrap(),
        PathBuf::from("/tmp/certify/.certify/732ead61db/bfcl-subset-echolp-b.log")
    );
}

#[test]
fn paths_that_escape_the_out_dir_are_refused() {
    let out = Path::new("/tmp/certify");
    for (name, rel) in [
        ("r.json", "/etc/r.json"),
        ("r.json", "../r.json"),
        ("r.json", ".benchmarks/../../r.json"),
        ("r.json", ".benchmarks/./r.json"),
        ("r.json", ""),
        ("r.json", ".benchmarks/r.json/"),
        ("r.json", ".benchmarks\\r.json"),
    ] {
        let m = meta(name, rel, b"x");
        let e = destination(out, &m).expect_err(rel);
        assert_eq!(e.obj.code, "bad_args", "{rel}");
    }
}

#[test]
fn verification_needs_both_the_size_and_the_hash() {
    let m = meta("r.json", ".benchmarks/g/r.json", b"hello");
    assert!(verify(&m, b"hello").is_ok());
    // NEGATIVE CONTROLS: wrong length; same length, different bytes.
    let e = verify(&m, b"hell").unwrap_err();
    assert!(
        e.obj.message.contains("4 bytes arrived, 5 promised"),
        "{}",
        e.obj.message
    );
    let e = verify(&m, b"hallo").unwrap_err();
    assert!(e.obj.message.contains("sha256"), "{}", e.obj.message);
    assert_eq!(e.exit, super::super::exit::Code::Usage);
}
