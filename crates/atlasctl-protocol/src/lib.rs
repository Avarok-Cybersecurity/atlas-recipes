// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! Wire types shared by the agent, the CLI, and the browser client.
//!
//! Deliberately dependency-light — serde and nothing else — so the entire
//! surface a webpage can reach is small enough to read in one sitting.

pub mod fleet;
pub mod id;
pub mod msg;
pub mod settings;
pub mod telemetry;

pub use fleet::{
    AlertKind, DisplayName, Launchability, LinkClass, Metric, NodeAddress, NodeAlert,
    NodeDescriptor, NodeId, NodeIdError, NodeVitals, PairingState, Severity,
};
pub use id::{RecipeId, RecipeIdError};
pub use msg::{AgentError, ClientMsg, RecipeInfo, RunningLaunch, ServerMsg};
pub use settings::{Bound, Group, SettingError, SettingSpec, SettingValue};
pub use telemetry::{DeviceStats, EngineStats, LaunchPhase, Stats, TelemetryCaps};

/// Protocol version this build speaks.
///
/// A client and an agent that disagree must say so at the handshake rather than
/// discovering it halfway through a launch.
///
/// * 1 — initial.
/// * 2 — pairing became two-phase. `PairPeer` runs the exchange and writes no
///   pin; `ConfirmPairing` establishes trust and `RejectPairing` discards it.
///   `PairResult.paired` became `PairResult.exchanged` because it no longer
///   means trusted, and `UnpairPeer` answers `PairDecision` rather than a
///   pairing-shaped reply. A version 1 page against a version 2 agent would
///   read `exchanged` as "trusted" and show a machine as paired that this
///   agent has not accepted, so the exact-match gate refusing it is the point
///   rather than an inconvenience.
/// * 3 — `PairPeerAt` added, so the browser can pair with a machine at an
///   address the operator typed. mDNS is link-local, so without it the browser
///   could only reach machines on the same broadcast domain. Additive, but the
///   handshake is an exact match by design, so it still takes a version.
/// * 4 — control verbs gained `on`, replies gained `on`/`via`,
///   `NodeDescriptor` gained `vouched_by`/`reached_via`, `PairingState`
///   gained `Vouched`, and the pairing verbs gained `allow_control`. A
///   version 3 page would render a vouched node with no provenance —
///   showing second-hand knowledge as first-hand is the exact lie the new
///   fields exist to prevent — so the exact-match gate refusing it is the
///   point.
pub const PROTOCOL_VERSION: u32 = 4;
