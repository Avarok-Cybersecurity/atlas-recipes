// SPDX-License-Identifier: AGPL-3.0-only

//! I/O boundaries.
//!
//! Business logic never performs I/O directly; it takes one of these traits and
//! the caller supplies either the real implementation or a recording mock. That
//! is what lets the whole launch path — resolution, translation, orchestration —
//! be tested with no GPU, no docker, and no network.

pub mod fs;
pub mod process;

pub use fs::{FileSystem, StdFileSystem};
pub use process::{Output, ProcessRunner, StdProcessRunner};

#[cfg(any(test, feature = "test-mocks"))]
pub use fs::MemFileSystem;
#[cfg(any(test, feature = "test-mocks"))]
pub use process::RecordingRunner;
