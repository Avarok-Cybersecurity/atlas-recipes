// SPDX-License-Identifier: AGPL-3.0-only

//! The node's capability report for a bench submitter: facts, read fresh.

use super::host::BenchHost;
use super::ports_std::Provenance;
use atlasctl_protocol::fleet::Metric;
use atlasctl_protocol::msg::bench::Sha;
use atlasctl_protocol::msg::bench_node::{BenchNodeInfo, BuiltSha, GpuInfo, RepoInfo};

/// `nvidia-smi` header line → the CUDA version it names (`13.0`).
fn cuda_version() -> String {
    std::process::Command::new("nvidia-smi")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let i = text.find("CUDA Version:")?;
            text[i + 13..].split_whitespace().next().map(str::to_owned)
        })
        .unwrap_or_default()
}

/// `nvidia-smi --query-gpu=name,driver_version,count`.
fn gpu_identity() -> Option<(String, String, u32)> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,driver_version,count",
            "--format=csv,noheader",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .to_string();
    let mut parts = line.split(',').map(str::trim);
    let name = parts.next()?.to_string();
    let driver = parts.next()?.to_string();
    let count = parts.next()?.parse().unwrap_or(1);
    Some((name, driver, count))
}

/// The signing identity Atlas would use: fingerprint = first 16 hex of
/// SHA-256(public key), the same rule `atlas-plugin`'s `signing.rs` applies.
fn signer(atlas_home: &std::path::Path) -> (Option<String>, Option<String>) {
    let pk8 = atlas_home.join("identity").join("ed25519.pk8");
    let Ok(bytes) = std::fs::read(&pk8) else {
        return (None, None);
    };
    use ed25519_dalek::pkcs8::DecodePrivateKey as _;
    let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_der(&bytes) else {
        return (None, None);
    };
    let public = key.verifying_key().to_bytes();
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(public);
    (Some(hex::encode(&digest[..8])), Some(hex::encode(public)))
}

fn built_shas(host: &BenchHost) -> Vec<BuiltSha> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(host.cfg.builds_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(e.path().join("provenance.json")) else {
            continue;
        };
        let Ok(p) = serde_json::from_str::<Provenance>(&text) else {
            continue;
        };
        out.push(BuiltSha {
            sha: p.sha,
            binary_sha256: p.binary_sha256,
            built_at_s: p.built_at_s,
            bytes: p.bytes,
        });
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.built_at_s));
    out
}

fn repo_info(host: &BenchHost) -> RepoInfo {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&host.cfg.atlas_repo)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    RepoInfo {
        path: host.cfg.atlas_repo.display().to_string(),
        remote_name: host.cfg.allowed_remote.clone(),
        remote_url: git(&["remote", "get-url", &host.cfg.allowed_remote]).unwrap_or_default(),
        head_sha: git(&["rev-parse", "HEAD"]).and_then(|s| Sha::parse(&s).ok()),
        fetched_at_s: std::fs::metadata(host.cfg.atlas_repo.join(".git").join("FETCH_HEAD"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()),
    }
}

/// Assemble the report.
pub fn node_info(host: &BenchHost) -> BenchNodeInfo {
    let local = crate::fleet::FleetView::nodes(host.fleet.as_ref())
        .into_iter()
        .find(|n| n.is_local);
    let vitals = local.as_ref().and_then(|n| n.vitals.clone());
    let alerts = local.as_ref().map(|n| n.alerts.clone()).unwrap_or_default();
    let name = local
        .as_ref()
        .map(|n| n.name.clone())
        .unwrap_or_else(|| atlasctl_protocol::fleet::DisplayName::new("unknown"));
    let gpu = gpu_identity().map(|(gname, driver, count)| GpuInfo {
        name: gname,
        count,
        driver_version: driver,
        cuda_version: cuda_version(),
        sm_clock_mhz: vitals
            .as_ref()
            .map_or(Metric::Unsupported, |v| v.sm_clock_mhz),
        sm_clock_healthy_mhz: vitals.as_ref().and_then(|v| v.sm_clock_healthy_mhz),
        temperature_c: vitals
            .as_ref()
            .map_or(Metric::Unsupported, |v| v.temperature_c),
        memory_total_bytes: vitals
            .as_ref()
            .map_or(Metric::Unsupported, |v| v.memory_total_bytes),
        memory_used_frac: vitals
            .as_ref()
            .map_or(Metric::Unsupported, |v| v.memory_used_frac),
        memory_is_unified: true,
    });
    let readings =
        super::exclusive::probe(&host.cfg.cache_dir, host.running().map(|j| j.to_string()));
    let busy = super::exclusive::judge(
        &readings,
        host.cfg.min_free_fraction,
        host.cfg.min_free_disk_bytes,
    );
    let (signer_fp, signer_pubkey_hex) = signer(&host.cfg.atlas_home);
    let running_job = host
        .running()
        .and_then(|id| host.store.load(&id).ok())
        .map(|j| j.summary());
    BenchNodeInfo {
        node: host.local,
        name,
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        peer_version_max: crate::peer::wire::PEER_PROTOCOL_MAX,
        bench_enabled: true,
        disabled_reason: None,
        gpu,
        alerts,
        hardware_class: Some(host.cfg.hardware.clone()),
        atlas_repo: Some(repo_info(host)),
        atlas_home: Some(host.cfg.atlas_home.display().to_string()),
        signer_fp,
        signer_pubkey_hex,
        recipes_synced: host
            .cfg
            .atlas_home
            .join("atlas-recipes")
            .join("index.json")
            .exists(),
        built_shas: built_shas(host),
        busy: !busy.is_empty(),
        busy_reason: (!busy.is_empty()).then(|| {
            busy.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        }),
        running_job,
        queued: host.queued().map_or(0, |q| q.len() as u32),
        queue_depth: host.cfg.queue_depth,
        disk_free_bytes: readings
            .disk_free_bytes
            .map_or(Metric::Unsupported, |b| Metric::Reading { value: b as f64 }),
        min_free_disk_bytes: host.cfg.min_free_disk_bytes,
        min_free_fraction: host.cfg.min_free_fraction,
        host_free_fraction: readings
            .free_fraction
            .map_or(Metric::Unsupported, |f| Metric::Reading { value: f }),
        max_run_s: host.cfg.max_run_s,
    }
}

/// The report for an agent with no `bench.yaml`.
pub fn disabled(
    local: atlasctl_protocol::fleet::NodeId,
    name: atlasctl_protocol::fleet::DisplayName,
    reason: &str,
) -> BenchNodeInfo {
    BenchNodeInfo {
        node: local,
        name,
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        peer_version_max: crate::peer::wire::PEER_PROTOCOL_MAX,
        bench_enabled: false,
        disabled_reason: Some(reason.into()),
        gpu: None,
        alerts: vec![],
        hardware_class: None,
        atlas_repo: None,
        atlas_home: None,
        signer_fp: None,
        signer_pubkey_hex: None,
        recipes_synced: false,
        built_shas: vec![],
        busy: false,
        busy_reason: None,
        running_job: None,
        queued: 0,
        queue_depth: 0,
        disk_free_bytes: Metric::Unsupported,
        min_free_disk_bytes: 0,
        min_free_fraction: 0.0,
        host_free_fraction: Metric::Unsupported,
        max_run_s: 0,
    }
}
