// SPDX-License-Identifier: AGPL-3.0-only

//! The reservation a rank holds between prepare and commit.
//!
//! Split from [`super`] for headroom, not because the file was over the cap --
//! it was at 495, five lines short. That is not a margin: main broke today
//! because two changes each landed under the cap and their merge did not, and
//! the failure surfaces on whichever pull request happens to be open next.

/// A rendered rank, held between prepare and commit.
pub(crate) struct Reservation {
    pub(crate) epoch: String,
    pub(crate) recipe: String,
    pub(crate) plan: atlasctl_core::docker::LaunchPlan,
    /// When this hold stops binding, computed FORWARD from creation.
    ///
    /// Without an expiry a reservation is immortal: the head that made it can
    /// close its tab, crash, or restart for an upgrade, and nothing here ever
    /// releases it -- so every later cluster launch on this machine is refused
    /// until someone restarts the agent by hand. On a fleet, all of them.
    ///
    /// Stored as a deadline rather than a creation time so that nothing ever
    /// computes `Instant::now() - d`, which PANICS on Windows when `d` exceeds
    /// the time since boot. A CI runner is minutes old; that panic broke main.
    pub(crate) expires: std::time::Instant,
}

/// How long a reservation outlives the head that made it.
///
/// It only has to cover prepare -> commit, which is a few round trips plus
/// whatever `docker rm -f` takes; commit consumes the reservation, so a live
/// launch is never at risk from this. Ten minutes is far past that and still
/// short enough that an operator who lost a head can simply try again rather
/// than ssh to every machine.
pub(crate) const RESERVATION_TTL: std::time::Duration = std::time::Duration::from_secs(600);
