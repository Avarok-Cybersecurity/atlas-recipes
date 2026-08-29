// SPDX-License-Identifier: AGPL-3.0-only

//! How a session actually starts, stops, and inspects launches.
//!
//! Behind a trait so the session's decisions — which are the security-relevant
//! part — are tested against a recording mock rather than a docker daemon.

use atlasctl_core::{Recipe, ScalarValue};
use atlasctl_protocol::msg::{AgentError, RunningLaunch};
use std::collections::BTreeMap;

/// A rendered command, for the client to inspect before committing.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The command, shell-quoted for display.
    pub command: String,
    /// Recipe settings this agent version does not understand.
    pub unapplied: Vec<String>,
}

/// A launch that started.
#[derive(Debug, Clone, PartialEq)]
pub struct Started {
    /// Container name.
    pub container: String,
    /// Where the model is served, when it serves.
    pub endpoint: Option<String>,
}

pub mod docker;

pub use docker::DockerLauncher;

/// Runs launches.
pub trait Launcher: Send + Sync {
    /// Render the command without running it.
    fn preview(
        &self,
        recipe: &Recipe,
        overrides: &BTreeMap<String, ScalarValue>,
    ) -> Result<Preview, AgentError>;

    /// Start a recipe.
    fn launch(
        &self,
        recipe: &Recipe,
        overrides: &BTreeMap<String, ScalarValue>,
    ) -> Result<Started, AgentError>;

    /// Stop a recipe by name.
    fn stop(&self, recipe: &str) -> Result<(), AgentError>;

    /// What is currently running.
    fn running(&self) -> Result<Vec<RunningLaunch>, AgentError>;
}

#[cfg(any(test, feature = "test-mocks"))]
mod mock {
    use super::*;
    use std::sync::Mutex;

    /// What a mock launcher was asked to do.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Call {
        /// A preview was rendered.
        Preview(String),
        /// A launch was started.
        Launch(String, BTreeMap<String, ScalarValue>),
        /// A launch was stopped.
        Stop(String),
        /// Running launches were listed.
        Running,
    }

    /// Records what the session asked for, and answers as scripted.
    #[derive(Debug, Default)]
    pub struct RecordingLauncher {
        calls: Mutex<Vec<Call>>,
        fail_launch: Mutex<Option<AgentError>>,
    }

    impl RecordingLauncher {
        /// A launcher that succeeds at everything.
        pub fn new() -> Self {
            Self::default()
        }

        /// Make the next launch fail.
        pub fn failing(error: AgentError) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_launch: Mutex::new(Some(error)),
            }
        }

        /// Everything it was asked to do, in order.
        pub fn calls(&self) -> Vec<Call> {
            self.calls.lock().expect("lock").clone()
        }

        /// Whether it was asked to launch anything at all.
        pub fn launched_anything(&self) -> bool {
            self.calls().iter().any(|c| matches!(c, Call::Launch(..)))
        }

        fn record(&self, c: Call) {
            self.calls.lock().expect("lock").push(c);
        }
    }

    impl Launcher for RecordingLauncher {
        fn preview(
            &self,
            recipe: &Recipe,
            _o: &BTreeMap<String, ScalarValue>,
        ) -> Result<Preview, AgentError> {
            self.record(Call::Preview(recipe.name.clone()));
            Ok(Preview {
                command: format!("docker run … {}", recipe.model),
                unapplied: vec![],
            })
        }

        fn launch(
            &self,
            recipe: &Recipe,
            o: &BTreeMap<String, ScalarValue>,
        ) -> Result<Started, AgentError> {
            self.record(Call::Launch(recipe.name.clone(), o.clone()));
            if let Some(e) = self.fail_launch.lock().expect("lock").take() {
                return Err(e);
            }
            Ok(Started {
                container: format!("atlas-{}", recipe.name),
                endpoint: Some("http://localhost:8888/v1".into()),
            })
        }

        fn stop(&self, recipe: &str) -> Result<(), AgentError> {
            self.record(Call::Stop(recipe.to_string()));
            Ok(())
        }

        fn running(&self) -> Result<Vec<RunningLaunch>, AgentError> {
            self.record(Call::Running);
            Ok(Vec::new())
        }
    }
}

#[cfg(any(test, feature = "test-mocks"))]
mod mock_arc {
    use super::*;
    use std::sync::Arc;

    /// A shared handle is a launcher too, so a test can keep inspecting a
    /// [`RecordingLauncher`] after handing ownership to a `ControlHost`.
    /// Test-only: production owners hold exactly one launcher each.
    impl<T: Launcher> Launcher for Arc<T> {
        fn preview(
            &self,
            recipe: &Recipe,
            o: &BTreeMap<String, ScalarValue>,
        ) -> Result<Preview, AgentError> {
            self.as_ref().preview(recipe, o)
        }
        fn launch(
            &self,
            recipe: &Recipe,
            o: &BTreeMap<String, ScalarValue>,
        ) -> Result<Started, AgentError> {
            self.as_ref().launch(recipe, o)
        }
        fn stop(&self, recipe: &str) -> Result<(), AgentError> {
            self.as_ref().stop(recipe)
        }
        fn running(&self) -> Result<Vec<RunningLaunch>, AgentError> {
            self.as_ref().running()
        }
    }
}

#[cfg(any(test, feature = "test-mocks"))]
pub use mock::{Call, RecordingLauncher};
