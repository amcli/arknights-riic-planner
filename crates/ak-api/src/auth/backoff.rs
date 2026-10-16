//! A growing wait between failed sign-ins to one username.
//!
//! [`FREE_FAILURES`] wrong passwords in a row cost nothing. Each one after
//! that makes the next attempt wait, [`FIRST_WAIT`] at first and twice as
//! long each time after, up to [`MAX_WAIT`]. An attempt during the wait is
//! answered `429` without its password being checked. No single wait is
//! longer than [`MAX_WAIT`]; the right password at the first attempt after
//! it signs in as usual and clears the count, and so does [`FORGET_AFTER`]
//! without a failure. Someone who keeps failing for a username does keep
//! that username waiting, though, since the wait belongs to the username,
//! not to whoever is guessing.
//!
//! An attempt is counted as a failure as soon as it is let through, and
//! uncounted if its password turns out right, so a burst of attempts sent at
//! once is held to the same allowance as attempts sent one by one.
//!
//! Failures are counted by username, in any case, whether or not an account
//! has it, so the answers say nothing about which accounts exist. The counts
//! live in memory, in a fixed table of [`SLOTS`] slots chosen by a hash of
//! the username under a key picked at random when the server starts: a
//! restart forgets them, several servers would each keep their own, and the
//! table never grows. Two usernames that share a slot share a count, which
//! can only make one of them wait longer, except that signing in clears the
//! slot for both.

use std::hash::{BuildHasher, RandomState};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Wrong passwords in a row before any wait.
pub const FREE_FAILURES: u32 = 5;
/// The wait after the first wrong password beyond [`FREE_FAILURES`].
pub const FIRST_WAIT: Duration = Duration::from_secs(1);
/// The longest wait.
pub const MAX_WAIT: Duration = Duration::from_secs(60);
/// Time without a failure after which a count is forgotten.
pub const FORGET_AFTER: Duration = Duration::from_secs(15 * 60);
/// Slots in the table: about 3 MiB.
pub const SLOTS: usize = 1 << 17;

#[derive(Debug, Clone, Copy)]
struct Failures {
    /// Failed attempts in a row, counting any still being checked.
    count: u32,
    /// When the latest was let through.
    last: Instant,
}

impl Failures {
    /// How long until the next attempt may be made.
    fn wait(&self, now: Instant) -> Duration {
        (self.last + wait_after(self.count)).saturating_duration_since(now)
    }

    fn forgotten(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) >= FORGET_AFTER
    }
}

/// The wait after `count` wrong passwords in a row.
fn wait_after(count: u32) -> Duration {
    match count.checked_sub(FREE_FAILURES + 1) {
        None => Duration::ZERO,
        Some(doublings) => FIRST_WAIT
            .checked_mul(1 << doublings.min(31))
            .map_or(MAX_WAIT, |wait| wait.min(MAX_WAIT)),
    }
}

/// Failed sign-ins by username.
#[derive(Debug)]
pub struct LoginBackoff {
    slots: Mutex<Box<[Option<Failures>]>>,
    key: RandomState,
}

impl Default for LoginBackoff {
    fn default() -> Self {
        LoginBackoff {
            slots: Mutex::new(vec![None; SLOTS].into_boxed_slice()),
            key: RandomState::new(),
        }
    }
}

impl LoginBackoff {
    fn lock(&self) -> MutexGuard<'_, Box<[Option<Failures>]>> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The slot `username` is counted in; usernames are unique regardless of
    /// case.
    fn slot(&self, username: &str) -> usize {
        // The mask keeps the index below SLOTS, which fits in usize.
        (self.key.hash_one(username.to_ascii_lowercase()) as usize) & (SLOTS - 1)
    }

    /// Lets a sign-in as `username` go ahead, counting it as a failure until
    /// [`succeeded`](Self::succeeded) says otherwise, or answers how long it
    /// must still wait.
    pub fn attempt(&self, username: &str, now: Instant) -> Result<(), Duration> {
        let index = self.slot(username);
        let mut slots = self.lock();
        let failures = slots[index]
            .filter(|f| !f.forgotten(now))
            .unwrap_or(Failures {
                count: 0,
                last: now,
            });
        let wait = failures.wait(now);
        if !wait.is_zero() {
            return Err(wait);
        }
        slots[index] = Some(Failures {
            count: failures.count.saturating_add(1),
            last: now,
        });
        Ok(())
    }

    /// Clears `username`'s count after the right password.
    pub fn succeeded(&self, username: &str) {
        let index = self.slot(username);
        self.lock()[index] = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn the_wait_starts_after_the_free_failures_and_doubles_to_a_cap() {
        let waits: Vec<u64> = (1..=14).map(|n| wait_after(n).as_secs()).collect();
        assert_eq!(waits, [0, 0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(wait_after(u32::MAX), MAX_WAIT);
    }

    #[test]
    fn failures_make_the_next_attempt_wait() {
        let backoff = LoginBackoff::default();
        let t0 = Instant::now();
        for i in 0..=FREE_FAILURES {
            assert_eq!(backoff.attempt("Doctor", t0 + SECOND * i), Ok(()), "{i}");
        }
        let t = t0 + SECOND * FREE_FAILURES;
        assert_eq!(backoff.attempt("DOCTOR", t), Err(SECOND));
        assert_eq!(backoff.attempt("doctor", t + SECOND / 2), Err(SECOND / 2));
        // A refused attempt is not counted: the wait is unchanged.
        assert_eq!(backoff.attempt("doctor", t + SECOND), Ok(()));
        assert_eq!(backoff.attempt("doctor", t + SECOND), Err(SECOND * 2));
        // Another username is untouched.
        assert_eq!(backoff.attempt("amiya", t + SECOND), Ok(()));
    }

    #[test]
    fn attempts_at_once_get_the_same_allowance() {
        let backoff = LoginBackoff::default();
        let now = Instant::now();
        let let_through = (0..20)
            .filter(|_| backoff.attempt("doctor", now).is_ok())
            .count();
        assert_eq!(let_through, FREE_FAILURES as usize + 1);
    }

    #[test]
    fn the_right_password_or_a_quiet_spell_clears_the_count() {
        let backoff = LoginBackoff::default();
        let t0 = Instant::now();
        for _ in 0..=FREE_FAILURES {
            backoff.attempt("doctor", t0).unwrap();
        }
        assert!(backoff.attempt("doctor", t0).is_err());
        backoff.succeeded("Doctor");
        for _ in 0..=FREE_FAILURES {
            assert_eq!(backoff.attempt("doctor", t0), Ok(()));
        }
        assert!(backoff.attempt("doctor", t0).is_err());
        // After a quiet spell, counting starts over.
        let later = t0 + FORGET_AFTER;
        for _ in 0..=FREE_FAILURES {
            assert_eq!(backoff.attempt("doctor", later), Ok(()));
        }
    }

    #[test]
    fn usernames_sharing_a_slot_share_a_count() {
        let backoff = LoginBackoff::default();
        let target = backoff.slot("doctor");
        let twin = (0..)
            .map(|i| format!("user{i}"))
            .find(|name| backoff.slot(name) == target)
            .unwrap();
        let now = Instant::now();
        for _ in 0..=FREE_FAILURES {
            backoff.attempt(&twin, now).unwrap();
        }
        assert!(backoff.attempt("doctor", now).is_err());
    }
}
