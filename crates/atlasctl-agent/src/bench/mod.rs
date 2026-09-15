// SPDX-License-Identifier: AGPL-3.0-only

//! Benchmark jobs: a `bench`-granted peer asks this node to build the
//! configured Atlas checkout at a commit, run one certification gate, and
//! hand the records back.
//!
//! * [`config`] — `bench.yaml`, no defaults for anything that names a path.
//! * [`job`] / [`journal`] — the on-disk truth: record, key index, events.
//! * [`ports`] — what a job needs from the machine, as a trait.
//! * [`machine`] — one job's life over [`ports::Ports`], testable dry.
//! * [`ports_std`] — the Linux ports: git, cargo, a process group, `/proc`.
//! * [`child`] — the gate child: pid identity, signals, output lines.
//! * [`exclusive`] — why the box is busy.
//! * [`thermal`] — the live facts an equivalence check reads.
//! * [`runner`] — the single worker, recovery after a restart, retention.
//! * [`host`] — what the peer-serving path calls.

pub mod child;
pub mod config;
pub mod exclusive;
pub mod host;
pub mod job;
pub mod journal;
pub mod lease;
pub mod machine;
pub mod nodeinfo;
pub mod ports;
pub mod ports_std;
pub mod runner;
pub mod thermal;

pub use config::BenchConfig;
pub use host::BenchHost;
