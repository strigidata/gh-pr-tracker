//! How often a transient failure is retried, and how long between tries.

use std::num::NonZeroU32;
use std::time::Duration;

/// Three attempts: the first try and two retries.
const DEFAULT_ATTEMPTS: NonZeroU32 = match NonZeroU32::new(3) {
    Some(attempts) => attempts,
    None => NonZeroU32::MIN,
};

/// How long to wait before the second attempt.
const DEFAULT_FIRST_BACKOFF: Duration = Duration::from_millis(500);

/// The longest this crate will ever wait between attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// How many times a transient failure is retried, and how long between tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryBudget {
    attempts: NonZeroU32,
    first_backoff: Duration,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self {
            attempts: DEFAULT_ATTEMPTS,
            first_backoff: DEFAULT_FIRST_BACKOFF,
        }
    }
}

impl RetryBudget {
    /// A budget of `attempts` tries, waiting `first_backoff` before the second.
    #[must_use]
    pub const fn new(attempts: NonZeroU32, first_backoff: Duration) -> Self {
        Self {
            attempts,
            first_backoff,
        }
    }

    /// Every wait this budget prescribes, in order, one per attempt.
    ///
    /// The first is [`Duration::ZERO`], because backoff is what happens after a
    /// failure and sleeping before the first request would add latency to every
    /// successful poll. Handing the caller a wait *per attempt* rather than an
    /// attempt count and a formula is what keeps the off-by-one out of the
    /// client: there is no arithmetic at the call site to get wrong.
    ///
    /// There is no jitter. Jitter exists to stop many clients retrying in
    /// lockstep, and this is one developer's machine polling one repository.
    pub(crate) fn schedule(self) -> impl Iterator<Item = Duration> {
        let first_backoff = self.first_backoff;
        (0..self.attempts.get()).map(move |failures| Self::backoff_after(first_backoff, failures))
    }

    /// How long to wait before an attempt that follows `failures` failures.
    ///
    /// Doubling is unbounded arithmetic, so both ends are closed here: the
    /// exponent saturates rather than overflowing, and the product is capped at
    /// [`MAX_BACKOFF`]. An overflow in a poll loop is a panic in a debug build
    /// and an absurd sleep in a release one.
    fn backoff_after(first_backoff: Duration, failures: u32) -> Duration {
        let Some(doublings) = failures.checked_sub(1) else {
            return Duration::ZERO;
        };
        first_backoff
            .checked_mul(2_u32.saturating_pow(doublings))
            .unwrap_or(MAX_BACKOFF)
            .min(MAX_BACKOFF)
    }
}

#[cfg(test)]
mod tests {
    use super::{RetryBudget, MAX_BACKOFF};
    use std::num::NonZeroU32;
    use std::time::Duration;

    /// A budget of `attempts` tries at the given backoff, or `None` if the
    /// count was not a number of attempts at all.
    fn budget(attempts: u32, first_backoff: Duration) -> Option<RetryBudget> {
        NonZeroU32::new(attempts).map(|attempts| RetryBudget::new(attempts, first_backoff))
    }

    /// Every wait the budget prescribes, in order.
    ///
    /// Empty if `attempts` was not a number of attempts at all, which no
    /// assertion below accepts.
    fn schedule(attempts: u32, first_backoff: Duration) -> Vec<Duration> {
        budget(attempts, first_backoff)
            .map(|budget| budget.schedule().collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_first_attempt_is_not_delayed() {
        // Backoff is what happens *after* a failure. Sleeping before the first
        // request would add latency to every successful poll.
        assert_eq!(
            schedule(3, Duration::from_millis(500)).first(),
            Some(&Duration::ZERO)
        );
    }

    #[test]
    fn there_is_one_wait_per_attempt() {
        for attempts in [1_u32, 2, 3, 7] {
            assert_eq!(
                schedule(attempts, Duration::from_millis(500)).len(),
                attempts as usize,
                "a budget of {attempts} attempts must prescribe {attempts} waits"
            );
        }
    }

    #[test]
    fn each_retry_waits_twice_as_long_as_the_one_before() {
        assert_eq!(
            schedule(4, Duration::from_millis(500)),
            vec![
                Duration::ZERO,
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
            ]
        );
    }

    #[test]
    fn a_single_attempt_prescribes_no_retry() {
        assert_eq!(
            schedule(1, Duration::from_millis(500)),
            vec![Duration::ZERO]
        );
    }

    #[test]
    fn the_wait_is_capped_rather_than_doubling_without_limit() {
        // Doubling is unbounded arithmetic. Left alone it overflows, and an
        // overflow in a poll loop is either a panic in debug or an absurd sleep
        // in release. Both are worse than waiting the cap.
        let waits = schedule(40, Duration::from_secs(1));
        assert!(
            waits.iter().all(|wait| *wait <= MAX_BACKOFF),
            "no wait may exceed the cap: {waits:?}"
        );
        assert_eq!(
            waits.last(),
            Some(&MAX_BACKOFF),
            "a long enough budget must reach the cap, or this test proves nothing"
        );
    }

    #[test]
    fn a_zero_backoff_budget_prescribes_no_waiting_at_all() {
        // This is what the client's own tests run with, so a test that exercises
        // the retry path does not spend real seconds sleeping.
        assert_eq!(
            schedule(3, Duration::ZERO),
            vec![Duration::ZERO, Duration::ZERO, Duration::ZERO]
        );
    }
}
