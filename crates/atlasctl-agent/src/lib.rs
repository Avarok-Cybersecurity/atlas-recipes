// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! The local agent: lets the Atlas website launch recipes on this machine.

/// The boxed future a `dyn`-used trait method returns.
///
/// Several traits here are held as trait objects (`dyn ControlRelay`,
/// `dyn RankTransport`, `dyn ClusterControl`, `dyn PeerPairing`), and an
/// `async fn` in a trait is not dyn-compatible. Boxing is the way those methods
/// get to `await` instead of blocking a runtime worker — and writing the full
/// `Pin<Box<dyn Future<Output = …> + Send + 'a>>` at every one of them is what
/// `clippy::type_complexity` objects to, correctly.
pub type BoxFut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub mod cluster;
pub mod clusterdriver;
pub mod control;
pub mod daemon;
pub mod discovery;
pub mod fabric;
pub mod fleet;
pub mod guard;
pub mod identity;
pub mod joining;
pub mod launcher;
pub mod launchstats;
pub mod logs;
pub mod pairing;
pub mod peer;
pub mod rank;
pub mod rendezvous;
pub mod server;
pub mod session;
pub mod telemetry;
pub mod token;
pub mod transport;

/// Port the browser control channel listens on.
pub const DEFAULT_PORT: u16 = 34333;
