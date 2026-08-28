// SPDX-License-Identifier: AGPL-3.0-only

//! The pure recipe-to-command translation.

use super::collective::CollectiveEnv;
use super::command::{DockerCommand, UserSpec};
use super::profile::{DeviceProfile, LaunchProfile};
use crate::chain::{Overrides, ResolvedConfig, UserConfig};
use crate::flags::{self, UnmappedKey};
use crate::host::HostSnapshot;
use crate::recipe::{NotLaunchable, Recipe};
use std::collections::BTreeMap;

/// Where the model cache is mounted inside the container.
const CONTAINER_HF_HOME: &str = "/cache/huggingface";

/// Default rendezvous port for a multi-node launch.
pub const DEFAULT_MASTER_PORT: u16 = 29500;

/// Label marking a container as ours, so it survives an agent restart.
pub const LABEL_MANAGED: &str = "io.atlasctl.managed";
/// Label recording which recipe produced a container.
pub const LABEL_RECIPE: &str = "io.atlasctl.recipe";

/// Which node of a launch this command is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// The whole model on one box.
    Solo,
    /// One rank of a multi-node launch.
    Rank {
        /// This node's rank; rank 0 is the head and serves the API.
        rank: u16,
        /// Total participating nodes.
        world_size: u16,
        /// Address every rank rendezvouses on.
        master_addr: String,
        /// Port every rank rendezvouses on.
        master_port: u16,
    },
}

/// A translated launch: the command, plus everything worth telling the user.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPlan {
    /// The command to run.
    pub docker: DockerCommand,
    /// Settings the flag table does not claim, and which will not be applied.
    pub unmapped: Vec<UnmappedKey>,
    /// Top-level recipe keys the schema does not name.
    pub unknown_keys: Vec<String>,
}

/// Why a recipe could not be translated into a command.
#[derive(Debug, thiserror::Error)]
pub enum TranslateError {
    /// The recipe cannot be launched at all.
    #[error("{name}: {source}")]
    NotLaunchable {
        /// Recipe name.
        name: String,
        /// The reason.
        #[source]
        source: NotLaunchable,
    },

    /// A recipe tried to set the operator's network-egress policy.
    #[error(
        "{name}: a recipe may not set {key}. Offline-by-default is the \
         operator's policy, not the recipe's — a recipe that could flip it \
         could also decide, silently, that this launch may reach the network"
    )]
    EgressOverride {
        /// Recipe name.
        name: String,
        /// The key it tried to set.
        key: String,
    },

    /// Two settings would render the same flag.
    #[error("{name}: {source}")]
    AliasConflict {
        /// Recipe name.
        name: String,
        /// The colliding pair.
        #[source]
        source: flags::AliasConflict,
    },

    /// A multi-node recipe was asked to run on one node, or the reverse.
    #[error(
        "{name}: recipe requires {required} nodes but placement supplies {supplied}. \
         Launching it on fewer would silently serve a different model than the \
         recipe describes."
    )]
    NodeCountMismatch {
        /// Recipe name.
        name: String,
        /// Nodes the recipe needs.
        required: u32,
        /// Nodes the placement offers.
        supplied: u32,
    },

    /// The rendezvous address is not an address.
    ///
    /// It arrives from the head over the peer channel and is the one value in a
    /// rank's command line that a rank cannot derive for itself. Everything
    /// else the head sends is bounded by the settings schema; without this,
    /// this one field was an unbounded string going straight into argv, which
    /// let a paired head append serve flags to another machine's command line.
    #[error(
        "{name}: rendezvous address {addr:?} is not an IP address. The head \
         chooses it from addresses this fleet actually reported, so anything \
         else means the plan did not come from where it claims."
    )]
    BadRendezvousAddress {
        /// Recipe name.
        name: String,
        /// What was offered.
        addr: String,
    },
}

/// The injected policy a launch runs under.
///
/// Bundled rather than passed as loose arguments so that adding a provider —
/// a telemetry probe, a storage profile — does not ripple through every call
/// site, and so the vendor-specific pieces travel together as one decision.
pub struct LaunchContext<'a> {
    /// Container isolation posture.
    pub profile: &'a LaunchProfile,
    /// How the accelerator is exposed.
    pub devices: &'a dyn DeviceProfile,
    /// Fabric tuning for multi-node launches.
    pub collective: &'a dyn CollectiveEnv,
}

/// Turn a recipe plus its context into the command it implies.
///
/// Pure by construction: every host fact arrives in `host`, so the same inputs
/// always produce the same bytes. That matters beyond tidiness — the website
/// prints this exact string, and the golden corpus asserts it.
pub fn translate(
    recipe: &Recipe,
    overrides: &Overrides,
    user_config: &UserConfig,
    host: &HostSnapshot,
    placement: &Placement,
    ctx: &LaunchContext<'_>,
) -> Result<LaunchPlan, TranslateError> {
    let LaunchContext {
        profile,
        devices,
        collective,
    } = *ctx;
    recipe
        .launchable()
        .map_err(|source| TranslateError::NotLaunchable {
            name: recipe.name.clone(),
            source,
        })?;

    check_node_count(recipe, placement)?;

    let resolved = ResolvedConfig::resolve(&recipe.defaults, user_config, overrides);
    flags::validate_no_alias_conflict(resolved.as_map()).map_err(|source| {
        TranslateError::AliasConflict {
            name: recipe.name.clone(),
            source,
        }
    })?;

    // A worker rank serves no API, so its port is forced rather than resolved.
    let skip: &[&str] = match placement {
        Placement::Rank { rank, .. } if *rank > 0 => &["port"],
        _ => &[],
    };
    let (mut serve_flags, unmapped) = flags::render(resolved.as_map(), skip);
    if let Placement::Rank {
        rank,
        world_size,
        master_addr,
        master_port,
    } = placement
    {
        if *rank > 0 {
            serve_flags.push("--port".into());
            serve_flags.push("0".into());
        }
        serve_flags.push("--rank".into());
        serve_flags.push(rank.to_string());
        serve_flags.push("--world-size".into());
        serve_flags.push(world_size.to_string());
        // Parsed, not trusted. This is the only value in a rank's command line
        // that comes from another machine, and it reaches argv directly.
        if master_addr.parse::<std::net::IpAddr>().is_err() {
            return Err(TranslateError::BadRendezvousAddress {
                name: recipe.name.clone(),
                addr: master_addr.clone(),
            });
        }
        serve_flags.push("--master-addr".into());
        serve_flags.push(master_addr.clone());
        serve_flags.push("--master-port".into());
        serve_flags.push(master_port.to_string());
    }

    let mut command = vec![
        "spark".to_string(),
        "serve".to_string(),
        recipe.model.clone(),
    ];
    command.extend(serve_flags);

    let docker = DockerCommand {
        detach: true,
        entrypoint: profile.entrypoint.map(str::to_string),
        privileged: profile.privileged,
        device_flags: devices.docker_flags(),
        ipc: profile.ipc.to_string(),
        shm_size: profile.shm_size.to_string(),
        network: profile.network.to_string(),
        user: Some(UserSpec {
            uid: host.uid,
            gid: host.gid,
        }),
        security_opts: profile
            .security_opts
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        cap_add: profile.cap_add.iter().map(|s| (*s).to_string()).collect(),
        ulimits: profile.ulimits.iter().map(|s| (*s).to_string()).collect(),
        devices: profile.devices.iter().map(|s| (*s).to_string()).collect(),
        memory: None,
        labels: vec![
            (LABEL_MANAGED.to_string(), "1".to_string()),
            (LABEL_RECIPE.to_string(), recipe.name.clone()),
        ],
        auto_remove: true,
        restart: None,
        name: container_name(&recipe.name, placement),
        env: build_env(recipe, host, placement, collective)?,
        volumes: BTreeMap::from([(host.hf_cache_dir.clone(), CONTAINER_HF_HOME.to_string())]),
        image: recipe.container.clone(),
        command,
    };

    Ok(LaunchPlan {
        docker,
        unmapped,
        unknown_keys: recipe.unknown_keys.clone(),
    })
}

/// Refuse a placement that does not match what the recipe needs.
fn check_node_count(recipe: &Recipe, placement: &Placement) -> Result<(), TranslateError> {
    let supplied = match placement {
        Placement::Solo => 1,
        Placement::Rank { world_size, .. } => u32::from(*world_size),
    };
    let required = recipe.topology.min_nodes;
    if supplied < required {
        return Err(TranslateError::NodeCountMismatch {
            name: recipe.name.clone(),
            required,
            supplied,
        });
    }
    Ok(())
}

/// Deterministic container name, unique per rank.
fn container_name(recipe: &str, placement: &Placement) -> String {
    match placement {
        Placement::Solo => format!("atlas-{recipe}"),
        Placement::Rank { rank, .. } => format!("atlas-{recipe}-rank{rank}"),
    }
}

/// Container environment: our standard block, the fabric block for a cluster
/// launch, then the recipe's own.
///
/// `$VAR` in a recipe's values expands against an ALLOWLIST of the host's
/// environment, never the whole of it, and a recipe cannot overwrite the
/// offline block above.
///
/// Both restrictions exist because a recipe is not ours. Recipes are fetched
/// from a remote index and the recipe also names the image it runs, so
/// expanding `${HF_TOKEN}` or `${AWS_SECRET_ACCESS_KEY}` against the agent's
/// real environment hands the operator's secrets to code the recipe author
/// chose — and setting `HF_HUB_OFFLINE=0` re-opens the network to carry them
/// out. Neither needs a bug elsewhere to work; the two together are just what
/// "the recipe wins on collision" meant.
///
/// No recipe in this repository references `$VAR` at all, and none sets the
/// offline keys, so this narrows a capability nothing shipped is using.
fn build_env(
    recipe: &Recipe,
    host: &HostSnapshot,
    placement: &Placement,
    collective: &dyn CollectiveEnv,
) -> Result<BTreeMap<String, String>, TranslateError> {
    let mut env = BTreeMap::from([
        // Offline by default: weights are pre-fetched, and a launch that
        // silently reaches out to the network is a launch you cannot reproduce.
        ("HF_HUB_OFFLINE".to_string(), "1".to_string()),
        ("TRANSFORMERS_OFFLINE".to_string(), "1".to_string()),
        ("HF_HOME".to_string(), CONTAINER_HF_HOME.to_string()),
    ]);
    // Fabric tuning applies only when there is a fabric to tune.
    if matches!(placement, Placement::Rank { .. }) {
        env.extend(collective.cluster_env());
    }
    for (k, v) in &recipe.env {
        // The offline block is this launcher's statement, not the recipe's.
        // Silently letting the more specific value win is right for tuning and
        // wrong for the switch that decides whether the container can reach
        // the network at all.
        // Refused, not dropped. Silently ignoring it would launch the
        // container under a policy the recipe author did not choose, which is
        // worse than either honouring or refusing — the author is owed an
        // answer either way.
        if EGRESS_ENV.contains(&k.as_str()) {
            return Err(TranslateError::EgressOverride {
                name: recipe.name.clone(),
                key: k.clone(),
            });
        }
        env.insert(k.clone(), expand_allowed(host, v));
    }
    Ok(env)
}

/// Keys that decide whether this launch may reach the network.
///
/// `HF_HOME` is deliberately NOT here: it is a path inside the container, not
/// a boundary, and a custom cache layout is a real thing a recipe may want.
/// Forbid what is dangerous, only what is dangerous.
const EGRESS_ENV: [&str; 2] = ["HF_HUB_OFFLINE", "TRANSFORMERS_OFFLINE"];

/// Host variables a recipe's `$VAR` may read.
///
/// An allowlist, not a deny list: the interesting names are the ones nobody
/// thought to forbid. Adding an entry is a security decision — it publishes
/// that variable's value to any image any recipe names — and belongs with the
/// same scrutiny as the settings deny list.
const PASSTHROUGH_ENV: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];

/// Expand `$VAR` against the passthrough allowlist only.
///
/// A name outside the list reads as unset, which `HostSnapshot::expand`
/// already renders as the empty string. That is the existing contract for an
/// unset variable — "deliberately not an error: failing a launch over an unset
/// optional would be worse" — so a recipe asking for something it may not have
/// behaves exactly as if the host did not have it.
fn expand_allowed(host: &HostSnapshot, raw: &str) -> String {
    let allowed = HostSnapshot {
        env: host
            .env
            .iter()
            .filter(|(k, _)| PASSTHROUGH_ENV.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        ..host.clone()
    };
    allowed.expand(raw)
}

#[cfg(test)]
#[path = "translate/env_tests.rs"]
mod env_tests;
#[cfg(test)]
mod tests;
