// SPDX-License-Identifier: AGPL-3.0-only

//! Turning a recipe into the `docker run` it implies.

pub mod collective;
pub mod command;
pub mod diagnose;
pub mod profile;
pub mod quote;
pub mod translate;

pub use collective::{CollectiveEnv, NcclRoce, NoCollectiveEnv};
pub use command::{DockerCommand, UserSpec};
pub use diagnose::{DOCKER_SETUP_DOCS, DockerFault};
pub use profile::{AmdDevices, DeviceProfile, LaunchProfile, NvidiaDevices, ROOTLESS_V1};
pub use translate::{LaunchContext, LaunchPlan, Placement, TranslateError, translate};
