//! Sliding-window rate limiter.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A sliding-window rate limiter that allows up to `limit` requests per `window`.
pub struct RateLimiter {
    limit: u32,
    window: Duration,
    timestamps: Mutex<VecDeque<Instant>>,
    /// Time offset added to `Instant::now()` (used for testing time advancement).
    time_offset: Mutex<Duration>,
}

impl RateLimiter {
    /// Create a new rate limiter allowing `limit` requests per `window_secs` seconds.
    pub fn new(limit: u32, window_secs: u64) -> Self {
        Self {
            limit,
            window: Duration::from_secs(window_secs),
            timestamps: Mutex::new(VecDeque::new()),
            time_offset: Mutex::new(Duration::ZERO),
        }
    }

    fn now(&self) -> Instant {
        let base = Instant::now();
        let offset = self.time_offset.lock().unwrap_or_else(|e| e.into_inner());
        base + *offset
    }

    /// Advance the internal clock by the given duration.
    ///
    /// This is primarily useful for testing time-dependent behavior without
    /// actually sleeping.
    pub fn advance_time(&self, d: Duration) {
        let mut offset = self.time_offset.lock().unwrap_or_else(|e| e.into_inner());
        *offset += d;
    }

    /// Try to acquire a permit. Returns `true` if the request is allowed,
    /// `false` if the rate limit has been exceeded.
    pub fn try_acquire(&self) -> bool {
        let now = self.now();
        let cutoff = now - self.window;
        let mut ts = self.timestamps.lock().unwrap_or_else(|e| e.into_inner());

        // Evict expired entries.
        while let Some(&front) = ts.front() {
            if front <= cutoff {
                ts.pop_front();
            } else {
                break;
            }
        }

        if (ts.len() as u32) < self.limit {
            ts.push_back(now);
            true
        } else {
            false
        }
    }
}

/// Create a rate limiter from an optional configuration value.
///
/// - `None` -> `None` (disabled)
/// - `Some(0)` -> `None` (disabled)
/// - `Some(negative)` -> `None` (disabled)
/// - `Some(positive)` -> `Some(RateLimiter)` with 60-second window
pub fn make_rate_limiter(limit: Option<i64>) -> Option<RateLimiter> {
    match limit {
        None => None,
        Some(n) if n <= 0 => None,
        Some(n) => Some(RateLimiter::new(n as u32, 60)),
    }
}
