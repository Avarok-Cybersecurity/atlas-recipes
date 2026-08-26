// SPDX-License-Identifier: AGPL-3.0-only

//! The window in which this machine will accept a new member.
//!
//! Pairing has always required a human, and it still does — what changes here
//! is *which* machine they are standing at. The original ceremony mints the
//! code on the machine being added, which proves someone walked over to it.
//! That is unusable from a browser: the operator is at the laptop, and the
//! thing they are adding is a headless box they may not have a screen for.
//!
//! So the direction inverts. The laptop mints, the human carries the code in
//! the command they paste on the other machine, and the ceremony runs with the
//! roles swapped. The security property is unchanged in the part that matters:
//! **a web page still cannot pair with anything on its own**, because the code
//! has to physically reach another machine, and only a human can do that.
//!
//! What the inversion does cost is that the code now sits in a shell history,
//! so the window is deliberately small and answers to all three of:
//!
//!   * an expiry, so an abandoned invitation closes itself;
//!   * single use, so a code that worked cannot work twice;
//!   * an attempt limit, so it cannot be guessed inside its own lifetime.
//!
//! 8 digits is 10^8; at three attempts per window that is not a space worth
//! searching, and the limiter closes the window rather than merely refusing —
//! there is no value in leaving a partially-guessed code alive.

use crate::pairing::{CODE_DIGITS, MAX_ATTEMPTS, PairingCode};
use anyhow::Result;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a minted invitation stays open.
///
/// Long enough to walk to another machine and paste a command, short enough
/// that a code left in a scrollback stops mattering quickly.
pub const JOIN_TTL: Duration = Duration::from_secs(10 * 60);

/// An outstanding invitation.
struct Pending {
    code: PairingCode,
    opened: Instant,
    attempts: u8,
}

/// This machine's join window. Closed unless a human has just opened it.
#[derive(Default)]
pub struct JoinWindow {
    inner: Mutex<Option<Pending>>,
}

impl std::fmt::Debug for JoinWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinWindow")
            .field("open", &self.is_open())
            .finish()
    }
}

impl JoinWindow {
    /// Open a window, replacing any invitation already outstanding.
    ///
    /// Replacing rather than refusing: an operator who asks again has decided
    /// the previous code is not going to be used, and leaving both alive would
    /// widen the window for no reason.
    pub fn mint(&self) -> Result<String> {
        let code = PairingCode::generate();
        let digits = code.as_str().to_owned();
        let mut slot = self.lock()?;
        *slot = Some(Pending {
            code,
            opened: Instant::now(),
            attempts: 0,
        });
        Ok(digits)
    }

    /// Close the window without using it.
    pub fn revoke(&self) {
        if let Ok(mut slot) = self.lock() {
            *slot = None;
        }
    }

    /// Whether an unexpired invitation is outstanding.
    ///
    /// This is what the TLS verifier consults, so it must be cheap and must
    /// never panic on a poisoned lock — a closed window is the safe answer to
    /// every question it cannot answer.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|s| s.as_ref().map(|p| p.opened.elapsed() < JOIN_TTL))
            .unwrap_or(false)
    }

    /// How long the current invitation has left, if any.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        self.inner
            .lock()
            .ok()?
            .as_ref()
            .and_then(|p| JOIN_TTL.checked_sub(p.opened.elapsed()))
    }

    /// The code to run the ceremony against, without consuming it.
    ///
    /// Not consumed here because the exchange can fail for reasons that are
    /// not an attack — a dropped connection, a mistyped digit — and burning the
    /// invitation on the first fumble would send the operator back to the
    /// browser for a new one. [`Self::attempt_failed`] is what makes repeated
    /// failure expensive.
    #[must_use]
    pub fn peek(&self) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .as_ref()
            .filter(|p| p.opened.elapsed() < JOIN_TTL)
            .map(|p| p.code.as_str().to_owned())
    }

    /// Record a failed exchange, closing the window at the attempt limit.
    pub fn attempt_failed(&self) {
        if let Ok(mut slot) = self.lock()
            && let Some(p) = slot.as_mut()
        {
            p.attempts = p.attempts.saturating_add(1);
            if p.attempts >= MAX_ATTEMPTS {
                *slot = None;
            }
        }
    }

    /// Close the window because it was used. Single use is the point.
    pub fn consume(&self) {
        self.revoke();
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Pending>>> {
        self.inner
            .lock()
            .map_err(|_| anyhow::anyhow!("the join window lock is poisoned"))
    }
}

/// Whether a string could be a join code at all.
#[must_use]
pub fn looks_like_code(s: &str) -> bool {
    s.len() == CODE_DIGITS && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_agent_is_not_accepting_anyone() {
        let w = JoinWindow::default();
        assert!(!w.is_open());
        assert!(w.peek().is_none());
        assert!(w.remaining().is_none());
    }

    #[test]
    fn a_minted_code_is_eight_digits_and_opens_the_window() {
        let w = JoinWindow::default();
        let code = w.mint().expect("mints");
        assert!(looks_like_code(&code), "{code}");
        assert!(w.is_open());
        assert_eq!(w.peek().as_deref(), Some(code.as_str()));
    }

    /// Single use. A code that worked must not work twice, or an invitation
    /// becomes a standing one.
    #[test]
    fn using_the_window_closes_it() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        w.consume();
        assert!(!w.is_open());
        assert!(w.peek().is_none());
    }

    /// 10^8 is only out of reach while the number of guesses is small.
    #[test]
    fn the_window_closes_after_the_attempt_limit() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        for _ in 0..MAX_ATTEMPTS {
            w.attempt_failed();
        }
        assert!(!w.is_open(), "a guessable window must close itself");
        assert!(w.peek().is_none());
    }

    /// But one fumble must not send the operator back to the browser.
    #[test]
    fn a_single_failure_leaves_the_window_open() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        w.attempt_failed();
        assert!(w.is_open());
    }

    /// Minting again means the operator gave up on the last code; leaving both
    /// alive would widen the window for nothing.
    #[test]
    fn minting_again_replaces_rather_than_adds() {
        let w = JoinWindow::default();
        let first = w.mint().expect("mints");
        let second = w.mint().expect("mints");
        assert_ne!(first, second);
        assert_eq!(w.peek().as_deref(), Some(second.as_str()));
    }

    #[test]
    fn revoking_closes_it() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        w.revoke();
        assert!(!w.is_open());
    }

    /// The attempt counter belongs to the invitation, not the agent: a new
    /// code starts with a full budget.
    #[test]
    fn a_new_code_gets_a_fresh_attempt_budget() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        w.attempt_failed();
        w.attempt_failed();
        w.mint().expect("mints again");
        w.attempt_failed();
        w.attempt_failed();
        assert!(w.is_open(), "the earlier failures must not carry over");
    }
}
