//! Bounded retries for idempotent calls: a transient failure is retried after a randomised, exponentially growing
//! pause ("full jitter"), up to a number of attempts and a total time; a permanent failure returns at once. Every
//! attempt carries its own timeout in the transport, so a call never waits longer than its attempts allow.

use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::constants::{
    RETRY_BASE_DELAY_MS, RETRY_MAX_ATTEMPTS, RETRY_MAX_DELAY_MS, RETRY_MAX_ELAPSED_SECS,
};
use crate::{Error, Result};

/// How often and for how long a call is retried.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    pub(crate) max_attempts: u32,
    pub(crate) base_delay: Duration,
    pub(crate) max_delay: Duration,
    pub(crate) max_elapsed: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: RETRY_MAX_ATTEMPTS,
            base_delay: Duration::from_millis(RETRY_BASE_DELAY_MS),
            max_delay: Duration::from_millis(RETRY_MAX_DELAY_MS),
            max_elapsed: Duration::from_secs(RETRY_MAX_ELAPSED_SECS),
        }
    }
}

impl RetryPolicy {
    /// The pause before attempt `retry + 1`: random between zero and `base × 2^(retry − 1)`, capped at `max_delay`.
    fn pause(&self, retry: u32) -> Duration {
        let factor = 1_u32
            .checked_shl(retry.saturating_sub(1))
            .unwrap_or(u32::MAX);
        let ceiling = self
            .base_delay
            .checked_mul(factor)
            .map_or(self.max_delay, |delay| delay.min(self.max_delay));
        let ceiling_ms = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(fastrand::u64(0..=ceiling_ms))
    }

    /// Runs `attempt` (given the attempt number, from 1) until it succeeds, fails permanently, or the policy is spent.
    ///
    /// # Errors
    /// A permanent failure as it came; [`Error::RetriesExhausted`] with the last transient one when the attempts or
    /// the time ran out.
    pub(crate) async fn run<T, F, Fut>(&self, mut attempt: F) -> Result<T>
    where
        F: FnMut(u32) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let started = Instant::now();
        let mut attempts: u32 = 0;
        loop {
            attempts = attempts.saturating_add(1);
            let error = match attempt(attempts).await {
                Ok(value) => return Ok(value),
                Err(error) if !error.is_retryable() => return Err(error),
                Err(error) => error,
            };
            let pause = self.pause(attempts);
            let within_time = started
                .elapsed()
                .checked_add(pause)
                .is_some_and(|resume| resume < self.max_elapsed);
            if attempts >= self.max_attempts || !within_time {
                return Err(Error::RetriesExhausted {
                    attempts,
                    last: Box::new(error),
                });
            }
            tracing::debug!(attempt = attempts, pause_ms = ?pause.as_millis(), error = %error, "retrying");
            tokio::time::sleep(pause).await;
        }
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
