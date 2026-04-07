//! Per-repo reindex state tracking and freshness gating.
//!
//! Provides [`ReindexStateManager`] for tracking reindex lifecycle
//! (start / done / failed) across multiple repos, with configurable
//! freshness mode (relaxed vs strict) and wait-for-fresh semantics.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

// ─── Public types ────────────────────────────────────────────────────────────

/// Freshness gating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessMode {
    Relaxed,
    Strict,
}

/// Snapshot of a repo's reindex status returned by [`ReindexStateManager::get_status`].
#[derive(Debug, Clone)]
pub struct ReindexStatus {
    pub reindex_in_progress: bool,
    pub index_stale: bool,
    pub stale_since_ms: Option<u64>,
    /// Only populated when `consecutive_failures >= 2`.
    pub reindex_error: Option<String>,
    /// Only populated when `consecutive_failures >= 2`.
    pub reindex_failures: Option<u32>,
}

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
    consecutive_failures: u32,
    deferred_generation: u64,
    last_error: Option<String>,
    /// `true` when the repo is "fresh" (event is set).
    event_set: bool,
}

impl RepoReindexState {
    fn new() -> Self {
        Self {
            inner: Mutex::new(RepoInner {
                reindexing: false,
                stale_since: None,
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
    states: Mutex<HashMap<String, Arc<RepoReindexState>>>,
    freshness_mode: RwLock<FreshnessMode>,
}

impl ReindexStateManager {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
            freshness_mode: RwLock::new(FreshnessMode::Relaxed),
        }
    }

    /// Get or create the per-repo state. Returns the same `Arc` for repeated calls
    /// with the same `repo`.
    pub fn get_state(&self, repo: &str) -> Arc<RepoReindexState> {
        let mut map = self.states.lock().unwrap();
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
    pub fn mark_reindex_start(&self, repo: &str) {
        let state = self.get_state(repo);
        let mut inner = state.inner.lock().unwrap();
        inner.reindexing = true;
        inner.event_set = false;
        if inner.stale_since.is_none() {
            inner.stale_since = Some(Instant::now());
        }
        inner.deferred_generation += 1;
    }

    /// Mark a reindex as successfully completed.
    ///
    /// - Clears `reindexing`, `stale_since`, `last_error`
    /// - Resets `consecutive_failures` to 0
    /// - Sets the event (unblocks waiters)
    pub fn mark_reindex_done(&self, repo: &str) {
        let state = self.get_state(repo);
        let mut inner = state.inner.lock().unwrap();
        inner.reindexing = false;
        inner.stale_since = None;
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
        let mut inner = state.inner.lock().unwrap();
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
        let inner = state.inner.lock().unwrap();

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
            reindex_error,
            reindex_failures,
        }
    }

    /// Returns `true` if any tracked repo currently has `reindexing == true`.
    pub fn is_any_reindex_in_progress(&self) -> bool {
        let map = self.states.lock().unwrap();
        for state in map.values() {
            let inner = state.inner.lock().unwrap();
            if inner.reindexing {
                return true;
            }
        }
        false
    }

    /// Set the freshness mode. Only `"relaxed"` and `"strict"` are valid.
    pub fn set_freshness_mode(&self, mode: &str) -> Result<(), String> {
        let m = match mode {
            "relaxed" => FreshnessMode::Relaxed,
            "strict" => FreshnessMode::Strict,
            other => return Err(format!("Invalid freshness mode: {other}")),
        };
        *self.freshness_mode.write().unwrap() = m;
        Ok(())
    }

    pub fn get_freshness_mode(&self) -> FreshnessMode {
        *self.freshness_mode.read().unwrap()
    }

    /// In relaxed mode returns `true` immediately. In strict mode blocks until
    /// the event is set or the timeout expires.
    pub fn await_freshness_if_strict(&self, repo: &str, timeout_ms: Option<u64>) -> bool {
        if self.get_freshness_mode() == FreshnessMode::Relaxed {
            return true;
        }
        let state = self.get_state(repo);
        let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000));
        let mut inner = state.inner.lock().unwrap();
        let deadline = Instant::now() + timeout;
        while !inner.event_set {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (guard, _timeout_result) = state.cvar.wait_timeout(inner, remaining).unwrap();
            inner = guard;
        }
        true
    }

    /// Wait for the repo to become fresh. For unknown (never-started) repos
    /// returns fresh immediately. After persistent failure (2+) returns
    /// `fresh=false` with `reason="reindex_failed"`.
    pub fn wait_for_fresh_result(&self, repo: &str, timeout_ms: Option<u64>) -> WaitResult {
        let start = Instant::now();

        // Check if we even have state for this repo
        {
            let map = self.states.lock().unwrap();
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
        let deadline = Instant::now() + timeout;

        let mut inner = state.inner.lock().unwrap();

        // Wait while event is not set (reindex in progress, no result yet)
        while !inner.event_set {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return WaitResult {
                    fresh: false,
                    waited_ms: start.elapsed().as_millis() as u64,
                    reason: Some("timeout".to_string()),
                    reindex_error: None,
                    reindex_failures: None,
                };
            }
            let (guard, _) = state.cvar.wait_timeout(inner, remaining).unwrap();
            inner = guard;
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
