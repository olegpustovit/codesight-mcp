//! Per-repo reindex state tracking.
//!
//! Provides [`ReindexStateManager`] for tracking reindex lifecycle
//! (start / done / failed) across multiple repos, with wait-for-fresh
//! semantics for operators that need to block until an in-flight reindex
//! settles. Read-path handlers never block — they annotate responses with
//! `STALE_DATA_WARNING` when a reindex is mid-flight.

use parking_lot::{Condvar, Mutex};
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ─── Public types ────────────────────────────────────────────────────────────

/// Snapshot of a repo's reindex status returned by [`ReindexStateManager::get_status`].
#[derive(Debug, Clone)]
pub struct ReindexStatus {
    pub reindex_in_progress: bool,
    pub index_stale: bool,
    pub stale_since_ms: Option<u64>,
    pub files_in_flight: Vec<String>,
    /// Only populated when `consecutive_failures >= 2`.
    pub reindex_error: Option<String>,
    /// Only populated when `consecutive_failures >= 2`.
    pub reindex_failures: Option<u32>,
}

/// Returned by [`ReindexStateManager::try_mark_reindex_start`] when another
/// reindex is already in flight on the same repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusyError {
    pub repo: String,
}

impl std::fmt::Display for BusyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "reindex already in progress for '{}'", self.repo)
    }
}

impl std::error::Error for BusyError {}

/// Result of [`ReindexStateManager::wait_for_fresh_result`].
#[derive(Debug, Clone)]
pub struct WaitResult {
    pub fresh: bool,
    pub waited_ms: u64,
    pub reason: Option<String>,
    pub reindex_error: Option<String>,
    pub reindex_failures: Option<u32>,
}

// ─── Internal per-repo state ─────────────────────────────────────────────────

/// Thread-safe per-repo reindex state.
pub struct RepoReindexState {
    inner: Mutex<RepoInner>,
    /// Condvar signalled on done or failed.
    cvar: Condvar,
}

struct RepoInner {
    reindexing: bool,
    stale_since: Option<Instant>,
    files_in_flight: Vec<String>,
    consecutive_failures: u32,
    deferred_generation: u64,
    last_error: Option<String>,
    /// `true` when the repo is "fresh" (event is set).
    event_set: bool,
}

impl RepoReindexState {
    /// Atomically claim the reindex slot. Returns `Err(())` if a reindex is
    /// already in progress. On success, flips `reindexing=true`, clears the
    /// event, bumps `deferred_generation`, and sets `stale_since` if unset.
    fn try_begin_reindex(&self, files_in_flight: &[String]) -> Result<(), ()> {
        let mut inner = self.inner.lock();
        if inner.reindexing {
            return Err(());
        }
        inner.reindexing = true;
        inner.event_set = false;
        inner.files_in_flight = files_in_flight.to_vec();
        if inner.stale_since.is_none() {
            inner.stale_since = Some(Instant::now());
        }
        inner.deferred_generation += 1;
        Ok(())
    }

    fn new() -> Self {
        Self {
            inner: Mutex::new(RepoInner {
                reindexing: false,
                stale_since: None,
                files_in_flight: Vec::new(),
                consecutive_failures: 0,
                deferred_generation: 0,
                last_error: None,
                event_set: true, // fresh by default
            }),
            cvar: Condvar::new(),
        }
    }
}

// ─── Manager ─────────────────────────────────────────────────────────────────

/// Thread-safe manager holding reindex states for multiple repos.
pub struct ReindexStateManager {
    states: Mutex<FxHashMap<String, Arc<RepoReindexState>>>,
}

impl ReindexStateManager {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(FxHashMap::default()),
        }
    }

    /// Get or create the per-repo state. Returns the same `Arc` for repeated calls
    /// with the same `repo`.
    pub fn get_state(&self, repo: &str) -> Arc<RepoReindexState> {
        let mut map = self.states.lock();
        map.entry(repo.to_string())
            .or_insert_with(|| Arc::new(RepoReindexState::new()))
            .clone()
    }

    /// Mark a reindex as started.
    ///
    /// - Sets `reindexing = true`
    /// - Clears the event (so waiters block)
    /// - Sets `stale_since` only if not already set
    /// - Increments `deferred_generation`
    pub fn mark_reindex_start(&self, repo: &str, files_in_flight: &[String]) {
        let state = self.get_state(repo);
        let mut inner = state.inner.lock();
        inner.reindexing = true;
        inner.event_set = false;
        inner.files_in_flight = files_in_flight.to_vec();
        if inner.stale_since.is_none() {
            inner.stale_since = Some(Instant::now());
        }
        inner.deferred_generation += 1;
    }

    /// Atomically claim the reindex slot for this repo. Returns
    /// [`BusyError`] if a reindex is already in flight — the state is left
    /// untouched in that case.
    pub fn try_mark_reindex_start(
        &self,
        repo: &str,
        files_in_flight: &[String],
    ) -> Result<(), BusyError> {
        let state = self.get_state(repo);
        state
            .try_begin_reindex(files_in_flight)
            .map_err(|()| BusyError {
                repo: repo.to_string(),
            })
    }

    /// Mark a reindex as successfully completed.
    ///
    /// - Clears `reindexing`, `stale_since`, `last_error`
    /// - Resets `consecutive_failures` to 0
    /// - Sets the event (unblocks waiters)
    pub fn mark_reindex_done(&self, repo: &str) {
        let state = self.get_state(repo);
        let mut inner = state.inner.lock();
        inner.reindexing = false;
        inner.stale_since = None;
        inner.files_in_flight.clear();
        inner.consecutive_failures = 0;
        inner.last_error = None;
        inner.event_set = true;
        state.cvar.notify_all();
    }

    /// Mark a reindex as failed.
    ///
    /// - Increments `consecutive_failures`
    /// - Stores the error
    /// - Clears `reindexing` but keeps `stale_since`
    /// - Sets the event (unblocks waiters)
    pub fn mark_reindex_failed(&self, repo: &str, error: &str) {
        let state = self.get_state(repo);
        let mut inner = state.inner.lock();
        inner.reindexing = false;
        inner.consecutive_failures += 1;
        inner.last_error = Some(error.to_string());
        inner.event_set = true;
        state.cvar.notify_all();
    }

    /// Return a snapshot of the repo's reindex status.
    ///
    /// Error and failure count are only exposed when `consecutive_failures >= 2`.
    pub fn get_status(&self, repo: &str) -> ReindexStatus {
        let state = self.get_state(repo);
        let inner = state.inner.lock();

        let stale_since_ms = inner
            .stale_since
            .map(|t| Instant::now().duration_since(t).as_millis() as u64);

        let (reindex_error, reindex_failures) = if inner.consecutive_failures >= 2 {
            (inner.last_error.clone(), Some(inner.consecutive_failures))
        } else {
            (None, None)
        };

        ReindexStatus {
            reindex_in_progress: inner.reindexing,
            index_stale: inner.stale_since.is_some(),
            stale_since_ms,
            files_in_flight: inner.files_in_flight.clone(),
            reindex_error,
            reindex_failures,
        }
    }

    /// Returns `true` if any tracked repo currently has `reindexing == true`.
    pub fn is_any_reindex_in_progress(&self) -> bool {
        !self.in_flight_repos().is_empty()
    }

    /// Snapshot the list of repos currently being reindexed.
    pub fn in_flight_repos(&self) -> Vec<String> {
        let map = self.states.lock();
        let mut out = Vec::new();
        for (repo, state) in map.iter() {
            let inner = state.inner.lock();
            if inner.reindexing {
                out.push(repo.clone());
            }
        }
        out.sort();
        out
    }

    /// Wait for the repo to become fresh. For unknown (never-started) repos
    /// returns fresh immediately. After persistent failure (2+) returns
    /// `fresh=false` with `reason="reindex_failed"`.
    pub fn wait_for_fresh_result(&self, repo: &str, timeout_ms: Option<u64>) -> WaitResult {
        let start = Instant::now();

        // Check if we even have state for this repo
        {
            let map = self.states.lock();
            if !map.contains_key(repo) {
                return WaitResult {
                    fresh: true,
                    waited_ms: 0,
                    reason: None,
                    reindex_error: None,
                    reindex_failures: None,
                };
            }
        }

        let state = self.get_state(repo);
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000));

        let mut inner = state.inner.lock();
        let result = state
            .cvar
            .wait_while_for(&mut inner, |i| !i.event_set, timeout);

        if result.timed_out() {
            return WaitResult {
                fresh: false,
                waited_ms: start.elapsed().as_millis() as u64,
                reason: Some("timeout".to_string()),
                reindex_error: None,
                reindex_failures: None,
            };
        }

        let waited_ms = start.elapsed().as_millis() as u64;

        // Event is set. Check if persistent failure.
        if inner.consecutive_failures >= 2 {
            return WaitResult {
                fresh: false,
                waited_ms,
                reason: Some("reindex_failed".to_string()),
                reindex_error: inner.last_error.clone(),
                reindex_failures: Some(inner.consecutive_failures),
            };
        }

        WaitResult {
            fresh: true,
            waited_ms,
            reason: None,
            reindex_error: None,
            reindex_failures: None,
        }
    }
}

impl Default for ReindexStateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_begin_reindex_succeeds_when_idle() {
        let mgr = ReindexStateManager::new();
        assert!(mgr.try_mark_reindex_start("alpha", &[]).is_ok());
        let status = mgr.get_status("alpha");
        assert!(status.reindex_in_progress);
        assert!(status.index_stale);
    }

    #[test]
    fn try_begin_reindex_returns_busy_when_reindexing() {
        let mgr = ReindexStateManager::new();
        mgr.try_mark_reindex_start("beta", &[])
            .expect("first claim");
        let err = mgr
            .try_mark_reindex_start("beta", &[])
            .expect_err("second claim must fail");
        assert_eq!(err.repo, "beta");
    }

    #[test]
    fn try_begin_reindex_recovers_after_done() {
        let mgr = ReindexStateManager::new();
        mgr.try_mark_reindex_start("gamma", &[])
            .expect("first claim");
        mgr.mark_reindex_done("gamma");
        assert!(
            mgr.try_mark_reindex_start("gamma", &[]).is_ok(),
            "second claim after done must succeed"
        );
    }

    #[test]
    fn try_begin_reindex_recovers_after_failed() {
        let mgr = ReindexStateManager::new();
        mgr.try_mark_reindex_start("delta", &[])
            .expect("first claim");
        mgr.mark_reindex_failed("delta", "boom");
        assert!(
            mgr.try_mark_reindex_start("delta", &[]).is_ok(),
            "second claim after failure must succeed"
        );
    }
}
