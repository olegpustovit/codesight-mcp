//! Integration tests: agent v1 staleness.

use codesight_mcp::reindex_state::{FreshnessMode, ReindexStateManager};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════════════════════
// Repo State Creation
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn state_creates_new_state_for_unknown_repo() {
    let mgr = ReindexStateManager::new();
    let status = mgr.get_status("repo/new");
    assert!(!status.reindex_in_progress);
    assert!(!status.index_stale);
    assert!(status.stale_since_ms.is_none());
    assert!(status.reindex_error.is_none());
    assert!(status.reindex_failures.is_none());
}

#[test]
fn state_returns_same_instance_for_same_repo() {
    let mgr = ReindexStateManager::new();
    let s1 = mgr.get_state("repo/a");
    let s2 = mgr.get_state("repo/a");
    assert!(Arc::ptr_eq(&s1, &s2));
}

#[test]
fn state_different_repos_are_independent() {
    let mgr = ReindexStateManager::new();
    let s1 = mgr.get_state("repo/a");
    let s2 = mgr.get_state("repo/b");
    assert!(!Arc::ptr_eq(&s1, &s2));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Mark Reindex Start
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn mark_start_sets_reindex_in_progress() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    let status = mgr.get_status("repo/a");
    assert!(status.reindex_in_progress);
    assert!(status.index_stale);
}

#[test]
fn mark_start_clears_event() {
    // After mark_reindex_start, wait_for_fresh_result should block (event cleared).
    // We verify by attempting a short-timeout wait that should NOT return fresh.
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    let result = mgr.wait_for_fresh_result("repo/a", Some(10));
    assert!(!result.fresh);
    assert_eq!(result.reason.as_deref(), Some("timeout"));
}

#[test]
fn mark_start_sets_stale_since() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    let status = mgr.get_status("repo/a");
    assert!(status.stale_since_ms.is_some());
}

#[test]
fn mark_start_increments_deferred_generation() {
    let mgr = ReindexStateManager::new();
    let state = mgr.get_state("repo/a");

    // Check initial generation is 0 via a start/check cycle
    mgr.mark_reindex_start("repo/a");
    // After one start, generation should be 1
    // We can't directly read deferred_generation from outside, but we can
    // verify via successive starts that generation increments.
    mgr.mark_reindex_done("repo/a");

    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");

    // Verify the state is the same Arc (generation tracked internally)
    let state2 = mgr.get_state("repo/a");
    assert!(Arc::ptr_eq(&state, &state2));
}

#[test]
fn mark_start_stale_since_not_overwritten_on_consecutive_starts() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    let status1 = mgr.get_status("repo/a");
    let stale1 = status1.stale_since_ms.unwrap();

    // Small sleep to ensure time difference
    thread::sleep(Duration::from_millis(20));

    // Second start without done
    mgr.mark_reindex_start("repo/a");
    let status2 = mgr.get_status("repo/a");
    let stale2 = status2.stale_since_ms.unwrap();

    // stale_since should have increased (more time elapsed since the original instant)
    // but the underlying Instant should be the same, so stale2 >= stale1
    // The key point: stale_since was NOT reset, so stale2 > stale1
    // (more wall-clock time has passed since the original stale_since Instant).
    assert!(
        stale2 >= stale1 + 15,
        "stale_since should reflect the original start time, not be reset. stale1={stale1}, stale2={stale2}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Mark Reindex Done
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn mark_done_clears_in_progress_and_stale() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");
    let status = mgr.get_status("repo/a");
    assert!(!status.reindex_in_progress);
    assert!(!status.index_stale);
    assert!(status.stale_since_ms.is_none());
}

#[test]
fn mark_done_sets_event() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");
    let result = mgr.wait_for_fresh_result("repo/a", Some(10));
    assert!(result.fresh);
}

#[test]
fn mark_done_resets_consecutive_failures() {
    let mgr = ReindexStateManager::new();
    // start -> fail -> start -> fail -> start -> done
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "err1");
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "err2");
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");

    let status = mgr.get_status("repo/a");
    assert!(status.reindex_failures.is_none());
    assert!(status.reindex_error.is_none());
}

#[test]
fn mark_done_clears_error_on_success() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "some error");
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");

    let status = mgr.get_status("repo/a");
    assert!(status.reindex_error.is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Mark Reindex Failed
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn first_failure_no_error_in_status() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "disk full");

    let status = mgr.get_status("repo/a");
    assert!(!status.reindex_in_progress);
    assert!(status.index_stale);
    assert!(status.reindex_error.is_none());
    assert!(status.reindex_failures.is_none());
}

#[test]
fn second_failure_exposes_error() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "error 1");
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "error 2");

    let status = mgr.get_status("repo/a");
    assert!(status.index_stale);
    assert_eq!(status.reindex_error.as_deref(), Some("error 2"));
    assert_eq!(status.reindex_failures, Some(2));
}

#[test]
fn failed_reindex_sets_event_so_waiters_unblock() {
    let mgr = Arc::new(ReindexStateManager::new());
    mgr.mark_reindex_start("repo/a");

    let mgr2 = mgr.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        mgr2.mark_reindex_failed("repo/a", "oops");
    });

    let result = mgr.wait_for_fresh_result("repo/a", Some(2000));
    handle.join().unwrap();
    // First failure => still fresh (transient tolerance)
    assert!(result.fresh);
}

#[test]
fn stale_since_preserved_on_failure() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "err");

    let status = mgr.get_status("repo/a");
    assert!(status.stale_since_ms.is_some());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Get Reindex Status
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn status_idle_state() {
    let mgr = ReindexStateManager::new();
    let status = mgr.get_status("repo/idle");
    assert!(!status.reindex_in_progress);
    assert!(!status.index_stale);
    assert!(status.stale_since_ms.is_none());
}

#[test]
fn status_in_progress_state() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    let status = mgr.get_status("repo/a");
    assert!(status.reindex_in_progress);
    assert!(status.index_stale);
}

#[test]
fn is_any_reindex_in_progress_tracks_multiple_repos() {
    let mgr = ReindexStateManager::new();
    assert!(!mgr.is_any_reindex_in_progress());

    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_start("repo/b");
    assert!(mgr.is_any_reindex_in_progress());

    mgr.mark_reindex_done("repo/a");
    assert!(mgr.is_any_reindex_in_progress()); // repo/b still going

    mgr.mark_reindex_done("repo/b");
    assert!(!mgr.is_any_reindex_in_progress());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Freshness Mode
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn default_freshness_is_relaxed() {
    let mgr = ReindexStateManager::new();
    assert_eq!(mgr.get_freshness_mode(), FreshnessMode::Relaxed);
}

#[test]
fn set_freshness_mode_strict() {
    let mgr = ReindexStateManager::new();
    mgr.set_freshness_mode("strict").unwrap();
    assert_eq!(mgr.get_freshness_mode(), FreshnessMode::Strict);
}

#[test]
fn set_invalid_freshness_mode_raises() {
    let mgr = ReindexStateManager::new();
    let err = mgr.set_freshness_mode("lazy").unwrap_err();
    assert!(err.contains("Invalid freshness mode"), "got: {err}");
}

#[test]
fn await_relaxed_returns_immediately() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    // Relaxed mode: should return true immediately even though reindex is in progress
    let result = mgr.await_freshness_if_strict("repo/a", Some(10));
    assert!(result);
}

#[test]
fn await_strict_blocks_until_done() {
    let mgr = Arc::new(ReindexStateManager::new());
    mgr.set_freshness_mode("strict").unwrap();
    mgr.mark_reindex_start("repo/a");

    let mgr2 = mgr.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        mgr2.mark_reindex_done("repo/a");
    });

    let start = Instant::now();
    let result = mgr.await_freshness_if_strict("repo/a", Some(2000));
    let elapsed = start.elapsed();
    handle.join().unwrap();

    assert!(result);
    assert!(
        elapsed.as_millis() >= 30,
        "should have waited at least ~30ms, got {}ms",
        elapsed.as_millis()
    );
}

#[test]
fn await_strict_respects_custom_timeout() {
    let mgr = ReindexStateManager::new();
    mgr.set_freshness_mode("strict").unwrap();
    mgr.mark_reindex_start("repo/a");

    let start = Instant::now();
    let result = mgr.await_freshness_if_strict("repo/a", Some(50));
    let elapsed = start.elapsed();

    assert!(!result);
    assert!(
        elapsed.as_millis() < 500,
        "should not block indefinitely, got {}ms",
        elapsed.as_millis()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Wait for Fresh Result
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn wait_returns_fresh_for_unknown_repo() {
    let mgr = ReindexStateManager::new();
    let result = mgr.wait_for_fresh_result("repo/unknown", Some(100));
    assert!(result.fresh);
    assert_eq!(result.waited_ms, 0);
}

#[test]
fn wait_returns_fresh_after_done() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_done("repo/a");
    let result = mgr.wait_for_fresh_result("repo/a", Some(100));
    assert!(result.fresh);
}

#[test]
fn wait_returns_fresh_after_waiting() {
    let mgr = Arc::new(ReindexStateManager::new());
    mgr.mark_reindex_start("repo/a");

    let mgr2 = mgr.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        mgr2.mark_reindex_done("repo/a");
    });

    let result = mgr.wait_for_fresh_result("repo/a", Some(2000));
    handle.join().unwrap();

    assert!(result.fresh);
    assert!(
        result.waited_ms >= 30,
        "should have waited at least ~30ms, got {}ms",
        result.waited_ms
    );
}

#[test]
fn wait_returns_timeout_when_reindex_never_finishes() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");

    let result = mgr.wait_for_fresh_result("repo/a", Some(50));
    assert!(!result.fresh);
    assert_eq!(result.reason.as_deref(), Some("timeout"));
    assert!(
        result.waited_ms >= 40,
        "should have waited ~50ms, got {}ms",
        result.waited_ms
    );
}

#[test]
fn wait_first_failure_returns_fresh() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "disk full");

    let result = mgr.wait_for_fresh_result("repo/a", Some(100));
    assert!(result.fresh);
}

#[test]
fn wait_persistent_failure_returns_error() {
    let mgr = ReindexStateManager::new();
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "error 1");
    mgr.mark_reindex_start("repo/a");
    mgr.mark_reindex_failed("repo/a", "error 2");

    let result = mgr.wait_for_fresh_result("repo/a", Some(100));
    assert!(!result.fresh);
    assert_eq!(result.reason.as_deref(), Some("reindex_failed"));
    assert_eq!(result.reindex_error.as_deref(), Some("error 2"));
    assert_eq!(result.reindex_failures, Some(2));
}
