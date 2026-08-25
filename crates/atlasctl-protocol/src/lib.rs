// SPDX-License-Identifier: AGPL-3.0-only
#![deny(warnings)]
#![deny(clippy::all)]

//! Wire types shared by the agent, the CLI, and the browser client.
//!
//! Deliberately dependency-light — serde and nothing else — so the entire
//! surface a webpage can reach is small enough to read in one sitting.

pub mod id;
pub mod msg;
pub mod settings;
pub mod telemetry;

pub use id::{RecipeId, RecipeIdError};
pub use msg::{AgentError, ClientMsg, RecipeInfo, RunningLaunch, ServerMsg};
pub use settings::{Bound, Group, SettingError, SettingSpec, SettingValue};
pub use telemetry::{DeviceStats, EngineStats, LaunchPhase, Stats, TelemetryCaps};

/// Protocol version this build speaks.
///
/// A client and an agent that disagree must say so at the handshake rather than
/// discovering it halfway through a launch.
pub const PROTOCOL_VERSION: u32 = 1;
