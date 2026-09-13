// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use atlasctl_protocol::fleet::NodeId;
use atlasctl_protocol::msg::bench_event::{ArtifactKind, VerdictKind};

#[test]
fn reconnect_budget_is_per_outage_with_a_capped_backoff() {
    let p = Reconnect {
        budget: Duration::from_secs(100),
    };
    let t0 = Instant::now();
    // Inside the budget: 2, 4, 8, 16, 30, 30, …
    let pauses: Vec<u64> = (0..7)
        .map(|n| {
            p.next_attempt(t0, t0 + Duration::from_secs(1), n)
                .unwrap()
                .as_secs()
        })
        .collect();
    assert_eq!(pauses, vec![2, 4, 8, 16, 30, 30, 30]);
    // At or past the budget: give up.
    assert!(
        p.next_attempt(t0, t0 + Duration::from_secs(100), 0)
            .is_none()
    );
    assert!(
        p.next_attempt(t0, t0 + Duration::from_secs(500), 0)
            .is_none()
    );
    // NEGATIVE CONTROL: a zero budget never retries, not even once.
    let zero = Reconnect {
        budget: Duration::ZERO,
    };
    assert!(zero.next_attempt(t0, t0, 0).is_none());
}

fn ev(seq: u64, kind: EventKind) -> BenchEvent {
    BenchEvent {
        job: JobId::parse("jb-1-deadbeef").unwrap(),
        seq,
        at_ms: 0,
        kind,
    }
}

#[test]
fn the_tally_keeps_the_verdict_and_artifacts_and_stops_at_done() {
    let mut t = Tally::default();
    assert!(t.note(&ev(1, EventKind::Queued { position: 0 })).is_none());
    assert!(
        t.note(&ev(
            2,
            EventKind::Verdict {
                verdict: Verdict {
                    kind: VerdictKind::Pass,
                    text: "x".into()
                }
            }
        ))
        .is_none()
    );
    let meta = ArtifactMeta {
        name: "r.json".into(),
        relative_path: ".benchmarks/decode-floor/r.json".into(),
        bytes: 3,
        sha256: "00".into(),
        kind: ArtifactKind::Record,
    };
    assert!(
        t.note(&ev(3, EventKind::Artifact { meta: meta.clone() }))
            .is_none()
    );
    let outcome = Outcome::Cancelled {
        by: NodeId::from_bytes([2; 32]),
    };
    assert_eq!(
        t.note(&ev(
            4,
            EventKind::Done {
                outcome: outcome.clone()
            }
        )),
        Some(outcome)
    );
    assert_eq!(t.verdict.as_ref().map(|v| v.kind), Some(VerdictKind::Pass));
    assert_eq!(t.artifacts, vec![meta]);
}
