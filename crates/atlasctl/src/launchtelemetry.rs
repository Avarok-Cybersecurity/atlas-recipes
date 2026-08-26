// SPDX-License-Identifier: AGPL-3.0-only

//! Sampling a launch this agent started.
//!
//! The port is read back from the **running container's own arguments**, not
//! from the recipe. A launch can override the port, and a sampler that trusted
//! the recipe would scrape whatever else happened to be on the default — and
//! then report another model's throughput under this one's name. Reading it
//! from the container that is actually running is the only source that cannot
//! be wrong.

use atlasctl_agent::launchstats::LaunchSampler;
use atlasctl_agent::session::LaunchTelemetry;
use atlasctl_core::docker::translate::LABEL_RECIPE;
use atlasctl_core::io::ProcessRunner;
use atlasctl_protocol::RecipeId;
use atlasctl_protocol::msg::LaunchReading;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Samples the local launch through the container runtime and loopback HTTP.
pub struct LocalLaunchTelemetry {
    runner: Arc<dyn ProcessRunner>,
    sampler: LaunchSampler,
    /// The recipe the sampler currently holds a previous scrape for.
    ///
    /// Kept so that sampling a *different* launch clears it: differencing one
    /// engine's counters against another's would report a nonsense rate.
    current: Mutex<Option<String>>,
}

impl std::fmt::Debug for LocalLaunchTelemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalLaunchTelemetry")
            .finish_non_exhaustive()
    }
}

impl LocalLaunchTelemetry {
    /// Build a sampler for launches on this machine.
    #[must_use]
    pub fn new(runner: Arc<dyn ProcessRunner>, sampler: LaunchSampler) -> Self {
        Self {
            runner,
            sampler,
            current: Mutex::new(None),
        }
    }

    /// The port a running launch is actually serving on.
    fn port_of(&self, recipe: &str) -> Result<u16, String> {
        let out = self
            .runner
            .run(&[
                "docker".into(),
                "ps".into(),
                "--filter".into(),
                format!("label={LABEL_RECIPE}={recipe}"),
                "--format".into(),
                "{{.Names}}".into(),
            ])
            .map_err(|e| format!("{e:#}"))?;
        let name = out
            .stdout
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .ok_or_else(|| format!("{recipe} is not running on this machine"))?;

        let args = self
            .runner
            .run(&[
                "docker".into(),
                "inspect".into(),
                "-f".into(),
                "{{range .Args}}{{println .}}{{end}}".into(),
                name.to_owned(),
            ])
            .map_err(|e| format!("{e:#}"))?;

        let mut tokens = args.stdout.lines().map(str::trim);
        while let Some(t) = tokens.next() {
            if t == "--port" {
                return tokens
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| format!("{recipe} has an unreadable --port"));
            }
            if let Some(v) = t.strip_prefix("--port=") {
                return v
                    .parse()
                    .map_err(|_| format!("{recipe} has an unreadable --port"));
            }
        }
        // Not a default: the launch either names a port or it is not one this
        // sampler can find, and guessing would scrape somebody else's server.
        Err(format!("{recipe} does not name a port to scrape"))
    }
}

impl LaunchTelemetry for LocalLaunchTelemetry {
    fn sample(&self, recipe: &RecipeId) -> Result<LaunchReading, String> {
        {
            let mut held = self.current.lock().expect("telemetry lock poisoned");
            if held.as_deref() != Some(recipe.as_str()) {
                // A different engine's counters are not a baseline for this one.
                self.sampler.reset();
                *held = Some(recipe.as_str().to_owned());
            }
        }
        let port = self.port_of(recipe.as_str())?;
        let s = self
            .sampler
            .sample(port, Instant::now())
            .map_err(|e| format!("{e:#}"))?;
        Ok(LaunchReading {
            requests_total: s.requests_total,
            requests_active: s.requests_active,
            decode_tokens_per_s: s.decode_tokens_per_s,
            prompt_tokens_per_s: s.prompt_tokens_per_s,
            ttft_p50_s: s.ttft_p50_s,
            ttft_p90_s: s.ttft_p90_s,
            accept_rate: s.accept_rate,
            prefix_hit_rate: s.prefix_hit_rate,
            window_s: s.window_s,
        })
    }
}
