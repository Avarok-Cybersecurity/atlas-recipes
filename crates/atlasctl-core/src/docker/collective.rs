// SPDX-License-Identifier: AGPL-3.0-only

//! The collective-communication environment for a multi-node launch.
//!
//! Kept behind a trait because it is the most vendor-specific thing a cluster
//! launch does: NVIDIA ranks talk over NCCL, AMD ranks over RCCL, and a host
//! with no usable fabric needs a TCP fallback rather than a pile of variables
//! naming hardware it does not have.

use std::collections::BTreeMap;

/// Supplies the environment ranks need to find and talk to each other.
pub trait CollectiveEnv: Send + Sync {
    /// Short identifier for display and logs.
    fn name(&self) -> &'static str;

    /// Variables to inject for a multi-node launch.
    ///
    /// Solo launches never call this: a single rank has nobody to rendezvous
    /// with, and injecting fabric tuning would be noise at best.
    fn cluster_env(&self) -> BTreeMap<String, String>;
}

/// NCCL over RoCEv2, tuned for GB10-class nodes.
///
/// Two entries are deliberately empty strings. That is not an oversight: they
/// *clear* values the image or host may otherwise supply, and dropping them
/// would silently re-enable whatever was inherited. Interface and HCA selection
/// are intentionally absent — the reference implementation left them commented
/// out, noting they should come from cluster configuration, and guessing a NIC
/// name is exactly the kind of silent wrongness this port exists to avoid.
#[derive(Debug, Clone, Copy, Default)]
pub struct NcclRoce;

impl CollectiveEnv for NcclRoce {
    fn name(&self) -> &'static str {
        "nccl-roce"
    }

    fn cluster_env(&self) -> BTreeMap<String, String> {
        [
            ("NCCL_IB_GID_INDEX", ""),
            ("NCCL_CROSS_NIC", ""),
            ("NCCL_DEBUG", "INFO"),
            ("NCCL_IB_DISABLE", "0"),
            ("NCCL_IB_ROCE_VERSION_NUM", "2"),
            ("NCCL_IB_ADDR_FAMILY", "AF_INET"),
            ("NCCL_IB_TIMEOUT", "22"),
            ("NCCL_IB_RETRY_CNT", "7"),
            ("NCCL_NET_GDR_LEVEL", "0"),
            ("NCCL_DMABUF_ENABLE", "0"),
            ("NCCL_NVLS_ENABLE", "0"),
            ("NCCL_CUMEM_HOST_ENABLE", "0"),
            ("NCCL_PROTO", "Simple"),
            ("NCCL_ALGO", "Ring"),
            ("NCCL_BUFFSIZE", "33554432"),
            ("NCCL_MIN_NCHANNELS", "1"),
            ("NCCL_MAX_NCHANNELS", "2"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }
}

/// No fabric tuning at all.
///
/// For single-node launches and for hosts whose interconnect we cannot classify:
/// letting the collective library use its own defaults is honest, whereas
/// asserting RoCE settings on a machine without RoCE is not.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCollectiveEnv;

impl CollectiveEnv for NoCollectiveEnv {
    fn name(&self) -> &'static str {
        "none"
    }

    fn cluster_env(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roce_table_matches_the_reference_shape() {
        assert_eq!(NcclRoce.cluster_env().len(), 17);
    }

    #[test]
    fn cleared_variables_survive_as_empty_strings() {
        // The subtle one: these must be *present and empty*, not absent.
        let env = NcclRoce.cluster_env();
        assert_eq!(env.get("NCCL_IB_GID_INDEX").map(String::as_str), Some(""));
        assert_eq!(env.get("NCCL_CROSS_NIC").map(String::as_str), Some(""));
    }

    #[test]
    fn no_interface_or_hca_is_guessed() {
        // Guessing a NIC name would produce a launch that looks fine and
        // silently uses the wrong fabric.
        let env = NcclRoce.cluster_env();
        for guessed in ["NCCL_SOCKET_IFNAME", "NCCL_IB_HCA", "GLOO_SOCKET_IFNAME"] {
            assert!(
                !env.contains_key(guessed),
                "{guessed} must come from cluster config"
            );
        }
    }

    #[test]
    fn the_null_backend_asserts_nothing() {
        assert!(NoCollectiveEnv.cluster_env().is_empty());
    }
}
