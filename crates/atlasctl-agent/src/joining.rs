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
            .and_then(|s| {
                s.as_ref()
                    // Exhausted counts as closed. The slot outlives the last
                    // reservation so that ceremony can still consume it, but
                    // the TLS gate must stop admitting strangers the moment
                    // the guesses run out, not one handshake later.
                    .map(|p| p.opened.elapsed() < JOIN_TTL && p.attempts < MAX_ATTEMPTS)
            })
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

    /// Reserve one attempt and return the code, or `None` if none remain.
    ///
    /// The reservation is the point, and it is why this is not a `peek`. The
    /// attempt budget used to be spent *after* a ceremony failed, which bounds
    /// nothing: every accepted connection is served on its own task, so N
    /// peers could each read the same live code and each run a full online
    /// guess before any of them reported failure. "Three attempts per window"
    /// was true only of attempts that happened one at a time, and an attacker
    /// is under no obligation to be polite. Charging here makes the bound hold
    /// against concurrency, and it also makes every early exit self-accounting
    /// — a ceremony that dies before it reports anything has still spent its
    /// guess, because it was charged before it began.
    ///
    /// The cost is that a genuine fumble now spends one of the three rather
    /// than being free. Three is still two more than a careful operator needs,
    /// and an invitation that cannot be exhausted is not a limit.
    #[must_use]
    pub fn begin_attempt(&self) -> Option<String> {
        let mut slot = self.lock().ok()?;
        let p = slot.as_mut()?;
        // An expired invitation is dead and can be cleared here: nobody holds a
        // reservation against it that is still worth anything.
        if p.opened.elapsed() >= JOIN_TTL {
            *slot = None;
            return None;
        }
        // An EXHAUSTED one is refused without clearing. A ceremony holding the
        // final reservation may still be running and entitled to consume it,
        // and emptying the slot underneath it made its `consume` report false —
        // so `serve_join` refused a legitimate pairing with "already used by
        // another machine" when no other machine was involved. Under handshake
        // overlap, which is exactly what the reservation design is for, any
        // late caller could evict the winner. Expiry still clears it.
        if p.attempts >= MAX_ATTEMPTS {
            return None;
        }
        p.attempts = p.attempts.saturating_add(1);
        // Deliberately NOT closed here even when this was the last permitted
        // attempt. The ceremony it belongs to has not run yet, and closing now
        // meant a peer that went on to SUCCEED found the slot already empty:
        // `consume` returned false and `serve_join` refused a legitimate
        // pairing with "that invitation was already used by another machine",
        // which was both wrong and alarming. The effective budget was two.
        //
        // Exhaustion is expressed by the counter instead — `is_open` reports
        // closed and the guard above refuses the next caller — so nothing
        // further can start while the last ceremony is still entitled to
        // finish.
        Some(p.code.as_str().to_owned())
    }

    /// Close the window because it was used, reporting whether *this* caller
    /// closed it.
    ///
    /// `false` means someone else already spent the invitation. A caller that
    /// has just completed a ceremony must then refuse the pairing: two peers
    /// racing the same code would otherwise both be admitted, and single use
    /// is the property the whole window exists to provide.
    #[must_use]
    pub fn consume(&self) -> bool {
        self.lock().map(|mut s| s.take().is_some()).unwrap_or(false)
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
        assert!(w.begin_attempt().is_none());
        assert!(w.remaining().is_none());
    }

    #[test]
    fn a_minted_code_is_eight_digits_and_opens_the_window() {
        let w = JoinWindow::default();
        let code = w.mint().expect("mints");
        assert!(looks_like_code(&code), "{code}");
        assert!(w.is_open());
        assert_eq!(w.begin_attempt().as_deref(), Some(code.as_str()));
    }

    /// Single use. A code that worked must not work twice, or an invitation
    /// becomes a standing one.
    #[test]
    fn using_the_window_closes_it() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        assert!(w.consume(), "the first caller spends the invitation");
        assert!(!w.is_open());
        assert!(w.begin_attempt().is_none());
    }

    /// Two ceremonies can complete against one code if they overlap. Only one
    /// may be admitted, and `consume` is how the loser finds out.
    #[test]
    fn only_the_first_consumer_may_admit_a_machine() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        assert!(w.consume(), "first ceremony wins the invitation");
        assert!(
            !w.consume(),
            "the second must be told the invitation was already spent, or one \
             code admits two machines"
        );
    }

    /// 10^8 is only out of reach while the number of guesses is small.
    #[test]
    fn the_window_closes_after_the_attempt_limit() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        for _ in 0..MAX_ATTEMPTS {
            let _ = w.begin_attempt();
        }
        assert!(!w.is_open(), "a guessable window must close itself");
        assert!(w.begin_attempt().is_none());
    }

    /// But one fumble must not send the operator back to the browser.
    #[test]
    fn a_single_failure_leaves_the_window_open() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        let _ = w.begin_attempt();
        assert!(w.is_open());
    }

    /// The bound must hold when the attempts are simultaneous, which is the
    /// only way an attacker would ever make them. Before the reservation moved
    /// into `begin_attempt`, every one of these threads got the live code
    /// because the counter was only touched after a ceremony reported failure.
    #[test]
    fn concurrent_attempts_cannot_exceed_the_budget() {
        use std::sync::Arc;
        let w = Arc::new(JoinWindow::default());
        w.mint().expect("mints");
        let handles: Vec<_> = (0..32)
            .map(|_| {
                let w = Arc::clone(&w);
                std::thread::spawn(move || usize::from(w.begin_attempt().is_some()))
            })
            .collect();
        let handed_out: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(
            handed_out,
            usize::from(MAX_ATTEMPTS),
            "32 racing peers must still only get {MAX_ATTEMPTS} guesses"
        );
        assert!(!w.is_open());
    }

    /// Minting again means the operator gave up on the last code; leaving both
    /// alive would widen the window for nothing.
    #[test]
    fn minting_again_replaces_rather_than_adds() {
        let w = JoinWindow::default();
        let first = w.mint().expect("mints");
        let second = w.mint().expect("mints");
        assert_ne!(first, second);
        assert_eq!(w.begin_attempt().as_deref(), Some(second.as_str()));
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
        let _ = w.begin_attempt();
        let _ = w.begin_attempt();
        w.mint().expect("mints again");
        let _ = w.begin_attempt();
        let _ = w.begin_attempt();
        assert!(w.is_open(), "the earlier failures must not carry over");
    }

    /// A ceremony that reserved the LAST permitted attempt must still be able
    /// to win it.
    #[test]
    fn the_final_permitted_attempt_can_still_pair() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        for _ in 0..(MAX_ATTEMPTS - 1) {
            assert!(
                w.begin_attempt().is_some(),
                "earlier attempts are permitted"
            );
        }
        assert!(
            w.begin_attempt().is_some(),
            "the last permitted attempt must still get the code"
        );
        assert!(
            w.consume(),
            "and succeeding on it must count as spending the invitation, not as \
             losing a race to another machine"
        );
    }

    /// A caller who arrives after the budget is spent must be refused without
    /// evicting the ceremony that is still using the invitation.
    #[test]
    fn a_late_caller_cannot_evict_the_winner() {
        let w = JoinWindow::default();
        w.mint().expect("mints");
        for _ in 0..MAX_ATTEMPTS {
            assert!(w.begin_attempt().is_some());
        }
        assert!(w.begin_attempt().is_none(), "over budget is refused");
        assert!(
            w.consume(),
            "the holder of the last reservation must still be able to spend it"
        );
    }
}
