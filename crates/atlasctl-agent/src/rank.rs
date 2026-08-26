// SPDX-License-Identifier: AGPL-3.0-only

//! What this machine will do when another machine asks it to be a rank.
//!
//! One trait rather than four, because these calls are one lifecycle and share
//! one piece of state — the reservation. Splitting them would let a caller hold
//! `prepare` from one implementation and `commit` from another, which is
//! exactly the mistake the two-phase protocol exists to prevent.
//!
//! ## Why commit carries only an epoch
//!
//! [`RankService::commit`] takes no assignment. It runs the command this
//! machine rendered and stored during [`prepare`](RankService::prepare), from
//! its own recipe, under its own settings validation. A head that is
//! compromised between the two phases therefore cannot substitute anything:
//! the worst it can do is start the launch the operator already previewed, or
//! not start it. That property is the entire reason the protocol has two
//! phases rather than one, and it is why the assignment must not be resent.
//!
//! Every method is infallible-by-return rather than by panic. A rank that
//! cannot answer is an ordinary state of a fleet — a machine gets switched off
//! mid-ceremony — and the head's rollback path is what handles it.

use crate::cluster::{PrepareReply, RankAssignment};
use anyhow::Result;

/// This machine, acting as one rank of somebody else's cluster.
pub trait RankService: Send + Sync {
    /// The command this rank would run, plus settings this machine's flag
    /// table does not claim.
    ///
    /// # Errors
    /// If the recipe is unknown here, differs from the head's, or cannot be
    /// translated for this machine.
    fn render(&self, assignment: &RankAssignment) -> Result<(String, Vec<String>)>;

    /// This machine's content hash for a recipe.
    ///
    /// The head asks so that it can send a hash the ranks can disagree with.
    ///
    /// # Errors
    /// If this machine does not have the recipe.
    fn content_hash(&self, recipe: &str) -> Result<String>;

    /// Validate and reserve, starting nothing.
    ///
    /// A refusal here is the normal way a cluster launch fails, and the reason
    /// is shown to the operator verbatim, so it must be worth reading.
    fn prepare(&self, epoch: &str, assignment: &RankAssignment) -> PrepareReply;

    /// Start what was prepared under this epoch.
    ///
    /// # Errors
    /// If no reservation is held for that epoch, or the container runtime
    /// refuses.
    fn commit(&self, epoch: &str) -> Result<String>;

    /// Stop a container this machine started as a rank.
    ///
    /// Needed because a cluster is either whole or absent: when one rank fails
    /// to start, the ranks that already started have to be stopped, or they sit
    /// waiting forever on a rendezvous that will never complete.
    ///
    /// # Errors
    /// If the container runtime refuses.
    fn stop(&self, container: &str) -> Result<()>;

    /// Release a reservation without starting anything.
    ///
    /// Deliberately returns nothing: rollback runs when something has already
    /// gone wrong, and a failure to release must not mask the original reason.
    /// A stale reservation is released by the next prepare regardless.
    fn abort(&self, epoch: &str);
}
