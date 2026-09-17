// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

/// The collector must return what THIS job wrote — not everything the gate's
/// directory gained recently.
///
/// On 2026-09-17 a six-shard campaign lost shard 4 of `bfcl-subset-echolp`:
/// shard 2 finished on the same node and wrote its record in the same second
/// the next job started, the old one-second slack in the mtime window kept it,
/// and the submitter refused a job that came back with two records for one
/// gate. Twenty-seven minutes of GPU, thrown away by a rounding allowance.
#[test]
fn a_job_collects_its_own_records_and_not_the_previous_jobs() {
    let dir = std::env::temp_dir().join(format!("collect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let gate = "bfcl-subset-echolp";
    let records = dir.join("wt").join(".benchmarks").join(gate);
    std::fs::create_dir_all(&records).unwrap();
    let write = |name: &str, body: &str| {
        std::fs::write(records.join(name), body).unwrap();
    };

    // The previous job's record is already there when this one starts.
    write("2026-09-17-abc-s2of6.json", "{\"shard\":2}");
    write("2026-09-17-abc-s2of6.json.sig", "sig2");

    let before = state(&records);
    assert_eq!(
        before.len(),
        2,
        "the snapshot must see the neighbour: {before:?}"
    );

    // This job writes its own, in the same second.
    write("2026-09-17-abc-s4of6.json", "{\"shard\":4}");
    write("2026-09-17-abc-s4of6.json.sig", "sig4");

    let got = collect(&dir.join("wt"), gate, &before, &dir.join("dest")).unwrap();
    let names: Vec<&str> = got.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["2026-09-17-abc-s4of6.json", "2026-09-17-abc-s4of6.json.sig"],
        "the neighbour's record must not be attributed to this job"
    );
    assert_eq!(got[0].kind, ArtifactKind::Record);
    assert_eq!(got[1].kind, ArtifactKind::Signature);
    assert_eq!(
        got[0].relative_path,
        format!(".benchmarks/{gate}/2026-09-17-abc-s4of6.json")
    );
    assert!(dir.join("dest").join("2026-09-17-abc-s4of6.json").exists());

    // A RE-RUN of the same shard rewrites the same filename: the set difference
    // alone would return nothing and the job would report "no record", so the
    // snapshot compares WHEN each file was written, not just which exist.
    let before2 = state(&records);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write("2026-09-17-abc-s4of6.json", "{\"shard\":4,\"rerun\":true}");
    let again = collect(&dir.join("wt"), gate, &before2, &dir.join("dest2")).unwrap();
    assert_eq!(
        again.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        vec!["2026-09-17-abc-s4of6.json"],
        "a rewritten record is this job's too"
    );

    // NEGATIVE CONTROL: nothing written, nothing collected — so the assertions
    // above are not passing because the collector returns everything.
    let before3 = state(&records);
    assert!(
        collect(&dir.join("wt"), gate, &before3, &dir.join("dest3"))
            .unwrap()
            .is_empty(),
        "a job that wrote no record must collect nothing"
    );

    // Non-record files in the same directory are never artifacts.
    write("notes.txt", "scratch");
    let before4 = state(&records);
    assert!(!before4.contains_key("notes.txt"));
    let _ = std::fs::remove_dir_all(&dir);
}
