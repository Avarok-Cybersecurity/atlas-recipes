// SPDX-License-Identifier: AGPL-3.0-only

//! The container isolation posture, and how the accelerator is exposed.
//!
//! These are split deliberately. The *security* posture is vendor-neutral and
//! lives in one reviewable constant; the *device* flags differ per accelerator
//! vendor and live behind a trait. Atlas targets NVIDIA GB10 today with AMD in
//! bring-up, so no vendor-specific literal may leak into the launch path.

/// How a container is given access to the accelerator.
///
/// One implementation per vendor. Nothing outside an implementation of this
/// trait may name `--gpus`, `/dev/kfd`, or any other vendor-specific device
/// flag — that rule is what keeps the launch path portable, and CI lints it.
pub trait DeviceProfile: Send + Sync {
    /// A short identifier for display and logs.
    fn vendor(&self) -> &'static str;

    /// The docker flags that expose this vendor's accelerator.
    fn docker_flags(&self) -> Vec<String>;
}

/// NVIDIA, via the NVIDIA container runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct NvidiaDevices;

impl DeviceProfile for NvidiaDevices {
    fn vendor(&self) -> &'static str {
        "nvidia"
    }

    fn docker_flags(&self) -> Vec<String> {
        vec!["--gpus".into(), "all".into()]
    }
}

/// AMD, via ROCm's kernel and render nodes.
#[derive(Debug, Clone, Copy, Default)]
pub struct AmdDevices;

impl DeviceProfile for AmdDevices {
    fn vendor(&self) -> &'static str {
        "amd"
    }

    fn docker_flags(&self) -> Vec<String> {
        vec![
            "--device".into(),
            "/dev/kfd".into(),
            "--device".into(),
            "/dev/dri".into(),
            "--group-add".into(),
            "video".into(),
        ]
    }
}

/// The vendor-neutral container posture a launch runs under.
///
/// One named value rather than defaults scattered across the launch path, so
/// the whole isolation story can be reviewed in one place. A recipe cannot
/// change any of it — the reference implementation let recipes override
/// isolation through an `executor_config` block, and atlasctl refuses that key
/// outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProfile {
    /// Name, so a rendered command can say which posture produced it.
    pub name: &'static str,
    /// `--ipc=` value.
    pub ipc: &'static str,
    /// `--network=` value.
    pub network: &'static str,
    /// `--shm-size=` value.
    pub shm_size: &'static str,
    /// Whether to pass `--privileged`.
    pub privileged: bool,
    /// `--security-opt` values.
    pub security_opts: &'static [&'static str],
    /// `--cap-add` values.
    pub cap_add: &'static [&'static str],
    /// `--ulimit` values.
    pub ulimits: &'static [&'static str],
    /// `--device` values that are not accelerator devices.
    pub devices: &'static [&'static str],
    /// Value for `--entrypoint`; `Some("")` clears the image's own entrypoint.
    pub entrypoint: Option<&'static str>,
}

/// The posture Atlas serves under on a GB10-class box, unprivileged.
///
/// Rationale for the parts that look unusual, taken from the reference:
/// `IPC_LOCK` plus `memlock=-1` unblock `ibv_reg_mr` for RDMA; `SYS_NICE` is
/// needed by the io_uring SQPOLL kernel thread; and the default seccomp profile
/// blocks `io_uring_*`, so the high-speed swap path runs unconfined. Notably
/// `privileged` is **false** — the reference defaulted it to true and only
/// dropped it in rootless mode; we never raise it.
pub const ROOTLESS_V1: LaunchProfile = LaunchProfile {
    name: "rootless-v1",
    ipc: "host",
    network: "host",
    shm_size: "32gb",
    privileged: false,
    security_opts: &["no-new-privileges", "seccomp=unconfined"],
    cap_add: &["IPC_LOCK", "SYS_NICE"],
    ulimits: &["memlock=-1:-1", "stack=67108864"],
    devices: &["/dev/infiniband"],
    entrypoint: Some(""),
};

#[cfg(test)]
mod tests {
    use super::*;

    /// The posture must never grant `--privileged`. This is a compile-time
    /// fact about a constant, so it is enforced at compile time: a change to
    /// `ROOTLESS_V1` fails the build rather than a test run.
    const _: () = assert!(!ROOTLESS_V1.privileged);

    #[test]
    fn the_default_posture_hardens_rather_than_relaxes() {
        assert!(ROOTLESS_V1.security_opts.contains(&"no-new-privileges"));
    }

    #[test]
    fn the_entrypoint_is_cleared_not_absent() {
        // The Atlas image ships its own entrypoint; an absent value would run
        // it instead of `spark serve`, so the empty string is meaningful.
        assert_eq!(ROOTLESS_V1.entrypoint, Some(""));
    }

    #[test]
    fn each_vendor_emits_only_its_own_device_flags() {
        assert_eq!(NvidiaDevices.docker_flags(), ["--gpus", "all"]);
        let amd = AmdDevices.docker_flags();
        assert!(amd.contains(&"/dev/kfd".to_string()));
        assert!(
            !amd.iter().any(|f| f == "--gpus"),
            "AMD must not emit an NVIDIA flag"
        );
    }

    #[test]
    fn the_neutral_profile_names_no_accelerator_device() {
        // The agnosticism invariant, asserted rather than trusted: nothing in
        // the shared posture may mention a vendor's device.
        let rendered = format!("{ROOTLESS_V1:?}");
        for forbidden in ["--gpus", "/dev/kfd", "/dev/dri", "nvidia", "rocm"] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden} leaked into the shared profile"
            );
        }
    }
}
