// SPDX-License-Identifier: AGPL-3.0-only

//! Rendering what *this* machine would run as one rank of a cluster.
//!
//! The head never renders another machine's command. It does not know which
//! recipe revision that machine has, what its flag table claims, or what
//! hardware it is exposing — so a preview the head invented would be a guess
//! presented as the thing that will execute. Each rank answers for itself,
//! through the same `translate()` the launcher uses, so preview and execution
//! cannot drift.
//!
//! The recipe hash is checked before anything is rendered. Two machines running
//! different revisions of the same recipe would otherwise launch two different
//! models and call it one cluster.

use anyhow::{Context, Result};
use atlasctl_agent::cluster::RankAssignment;
use atlasctl_agent::daemon::RankRenderer;
use atlasctl_core::chain::UserConfig;
use atlasctl_core::docker::collective::CollectiveEnv;
use atlasctl_core::docker::profile::{DeviceProfile, LaunchProfile};
use atlasctl_core::docker::translate::{Placement, translate};
use atlasctl_core::host::HostSnapshot;
use atlasctl_core::registry::{RecipeRef, RegistrySet};
use atlasctl_core::settings;

/// Renders a rank from this machine's own recipe inventory.
pub struct LocalRankRenderer {
    registry: RegistrySet,
    host: HostSnapshot,
    profile: &'static LaunchProfile,
    devices: Box<dyn DeviceProfile>,
    collective: Box<dyn CollectiveEnv>,
}

impl std::fmt::Debug for LocalRankRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRankRenderer").finish_non_exhaustive()
    }
}

impl LocalRankRenderer {
    /// Build a renderer for this machine.
    #[must_use]
    pub fn new(
        registry: RegistrySet,
        host: HostSnapshot,
        profile: &'static LaunchProfile,
        devices: Box<dyn DeviceProfile>,
        collective: Box<dyn CollectiveEnv>,
    ) -> Self {
        Self {
            registry,
            host,
            profile,
            devices,
            collective,
        }
    }
}

impl RankRenderer for LocalRankRenderer {
    fn render(&self, assignment: &RankAssignment) -> Result<(String, Vec<String>)> {
        let recipe = self
            .registry
            .resolve(&RecipeRef::parse(&assignment.recipe))
            .with_context(|| format!("this node does not ship recipe {}", assignment.recipe))?;

        // The wire carries strings, and they go back through the SAME bounded
        // schema the browser's settings go through. A peer cannot widen what a
        // setting may be simply by virtue of being a peer.
        let overrides = settings::validate(&assignment.settings).map_err(|errors| {
            anyhow::anyhow!(
                "this node will not accept those settings: {}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })?;

        let placement = Placement::Rank {
            rank: assignment.rank,
            world_size: assignment.world_size,
            master_addr: assignment.master_addr.clone(),
            master_port: assignment.master_port,
        };

        let plan = translate(
            &recipe,
            &overrides,
            &UserConfig::default(),
            &self.host,
            &placement,
            &atlasctl_core::docker::translate::LaunchContext {
                profile: self.profile,
                devices: self.devices.as_ref(),
                collective: self.collective.as_ref(),
            },
        )
        .with_context(|| {
            format!(
                "rendering rank {} of {}",
                assignment.rank, assignment.recipe
            )
        })?;

        // Surfaced rather than swallowed: a setting this node's flag table does
        // not claim will silently not apply, and the operator should see that
        // before they commit rather than wonder afterwards.
        let unmapped = plan
            .unmapped
            .iter()
            .map(|u| u.key.clone())
            .collect::<Vec<_>>();

        Ok((plan.docker.to_string(), unmapped))
    }
}
