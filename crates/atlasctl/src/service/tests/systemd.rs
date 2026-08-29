// SPDX-License-Identifier: AGPL-3.0-only

//! The systemd plan's activate/verify contract, split from [`super`] for size
//! and to sit beside its Windows counterpart in `windows.rs`.

use crate::service::plan::{ServiceKind, plan};
use std::path::Path;

use super::{agent, home};

/// The Linux counterpart of the Windows verify test. `systemctl restart` returns
/// once ExecStart is SPAWNED for a Type=simple unit, and the agent then checks
/// its config dir, loads its token, probes docker and binds its port -- so every
/// failure this check exists to name happens after an instantaneous read. The
/// install said "installed and started" and the operator paired a browser
/// against a crash loop.
#[test]
fn a_systemd_verify_waits_and_then_confirms() {
    let p = plan(ServiceKind::Systemd, &agent(), &home(), 1000);
    let v = p.verify.join(" ");
    assert!(v.contains("is-active"), "{v}");
    assert!(
        v.contains("sleep"),
        "an instantaneous read races ExecStart: {v}"
    );
    // Two reads, not one: a single delayed read still blesses a unit on its way
    // down, which is exactly what a crash loop looks like at any one instant.
    assert_eq!(
        v.matches("is-active").count(),
        2,
        "one read cannot tell 'up' from 'up so far': {v}"
    );
}

/// systemd needs none of this — `enable --now` is idempotent and restarts a
/// live unit with the new binary. Adding a teardown there would stop an agent
/// that did not need stopping.
#[test]
fn systemd_needs_no_pre_step() {
    let p = plan(ServiceKind::Systemd, &agent(), Path::new("/home/x"), 1000);
    assert!(p.pre_activate.is_empty(), "{:?}", p.pre_activate);
}
