//! Integration tests: agent v1 caching.

use codesight_engine::rate_limit::{RateLimiter, make_rate_limiter};
use codesight_engine::result_cache::ResultCache;
use serde_json::json;
use std::time::Duration;

// ═══════════════════════════════════════════════════════════════════════════════
// Result Cache: Basic Get/Put/Miss
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cache_miss_returns_none() {
    let cache = ResultCache::new(256);
    let result = cache.get("evaluate_impact", "owner/repo", "(\"sym\", 1, 0)");
    assert!(result.is_none());
}

#[test]
fn cache_put_then_get_returns_payload() {
    let cache = ResultCache::new(256);
    let payload = json!({"impact": "high"});
    cache.put(
        "evaluate_impact",
        "owner/repo",
        "(\"sym\", 1, 0)",
        payload.clone(),
    );
    let result = cache.get("evaluate_impact", "owner/repo", "(\"sym\", 1, 0)");
    assert_eq!(result, Some(payload));
}

#[test]
fn cache_different_tools_dont_collide() {
    let cache = ResultCache::new(256);
    let a = json!({"tool": "a"});
    let b = json!({"tool": "b"});
    cache.put("tool_a", "owner/repo", "key", a.clone());
    cache.put("tool_b", "owner/repo", "key", b.clone());
    assert_eq!(cache.get("tool_a", "owner/repo", "key"), Some(a));
    assert_eq!(cache.get("tool_b", "owner/repo", "key"), Some(b));
}

#[test]
fn cache_different_sources_dont_collide() {
    let cache = ResultCache::new(256);
    let v1 = json!({"src": 1});
    let v2 = json!({"src": 2});
    cache.put("tool", "owner/repo1", "key", v1.clone());
    cache.put("tool", "owner/repo2", "key", v2.clone());
    assert_eq!(cache.get("tool", "owner/repo1", "key"), Some(v1));
    assert_eq!(cache.get("tool", "owner/repo2", "key"), Some(v2));
}

#[test]
fn cache_different_specific_keys_dont_collide() {
    let cache = ResultCache::new(256);
    let v1 = json!({"k": 1});
    let v2 = json!({"k": 2});
    cache.put("tool", "owner/repo", "(\"sym\", 1, 0)", v1.clone());
    cache.put("tool", "owner/repo", "(\"sym\", 2, 0)", v2.clone());
    assert_eq!(cache.get("tool", "owner/repo", "(\"sym\", 1, 0)"), Some(v1));
    assert_eq!(cache.get("tool", "owner/repo", "(\"sym\", 2, 0)"), Some(v2));
}

// ═══════════════════════════════════════════════════════════════════════════════
// Result Cache: Hit/Miss Counters and Stats
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cache_miss_increments_misses() {
    let cache = ResultCache::new(256);
    let _ = cache.get("tool", "owner/repo", "key");
    let s = cache.stats();
    assert_eq!(s.total_misses, 1);
    assert_eq!(s.total_hits, 0);
    assert!((s.hit_rate - 0.0).abs() < f64::EPSILON);
}

#[test]
fn cache_hit_increments_hits() {
    let cache = ResultCache::new(256);
    cache.put("tool", "owner/repo", "key", json!(1));
    let _ = cache.get("tool", "owner/repo", "key");
    let s = cache.stats();
    assert_eq!(s.total_hits, 1);
    assert_eq!(s.total_misses, 0);
    assert!((s.hit_rate - 1.0).abs() < f64::EPSILON);
}

#[test]
fn cache_mixed_hit_rate() {
    let cache = ResultCache::new(256);
    cache.put("tool", "owner/repo", "key", json!(1));
    let _ = cache.get("tool", "owner/repo", "key"); // hit
    let _ = cache.get("tool", "owner/repo", "miss"); // miss
    let s = cache.stats();
    assert_eq!(s.total_hits, 1);
    assert_eq!(s.total_misses, 1);
    assert!((s.hit_rate - 0.5).abs() < f64::EPSILON);
}

#[test]
fn cache_by_tool_breakdown() {
    let cache = ResultCache::new(256);
    cache.put("find_references", "owner/repo", "key", json!(1));
    let _ = cache.get("find_references", "owner/repo", "key"); // hit
    let _ = cache.get("find_references", "owner/repo", "miss"); // miss
    let s = cache.stats();
    let tool = s
        .by_tool
        .get("find_references")
        .expect("tool stats present");
    assert_eq!(tool.hits, 1);
    assert_eq!(tool.misses, 1);
    assert!((tool.hit_rate() - 0.5).abs() < f64::EPSILON);
}

#[test]
fn cache_entries_count() {
    let cache = ResultCache::new(256);
    cache.put("tool", "owner/repo", "k1", json!(1));
    cache.put("tool", "owner/repo", "k2", json!(2));
    let s = cache.stats();
    assert_eq!(s.cached_entries, 2);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Result Cache: Invalidation
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cache_invalidate_all() {
    let cache = ResultCache::new(256);
    cache.put("tool", "o/r1", "k1", json!(1));
    cache.put("tool", "o/r2", "k2", json!(2));
    let evicted = cache.invalidate(None);
    assert_eq!(evicted, 2);
    assert!(cache.get("tool", "o/r1", "k1").is_none());
    assert!(cache.get("tool", "o/r2", "k2").is_none());
}

#[test]
fn cache_invalidate_specific_source() {
    let cache = ResultCache::new(256);
    cache.put("tool", "o/r1", "k1", json!(1));
    cache.put("tool", "o/r2", "k2", json!(2));
    let evicted = cache.invalidate(Some("o/r1"));
    assert_eq!(evicted, 1);
    assert!(cache.get("tool", "o/r1", "k1").is_none());
    // o/r2 survives -- but calling get records a hit, so let's check via stats
    let cache2 = ResultCache::new(256);
    cache2.put("tool", "o/r1", "k1", json!(1));
    cache2.put("tool", "o/r2", "k2", json!(2));
    cache2.invalidate(Some("o/r1"));
    assert_eq!(cache2.get("tool", "o/r2", "k2"), Some(json!(2)));
}

#[test]
fn cache_invalidate_nonexistent_source_returns_zero() {
    let cache = ResultCache::new(256);
    cache.put("tool", "o/r1", "k1", json!(1));
    let evicted = cache.invalidate(Some("o/does_not_exist"));
    assert_eq!(evicted, 0);
    // existing entry untouched
    assert_eq!(cache.get("tool", "o/r1", "k1"), Some(json!(1)));
}

#[test]
fn cache_invalidate_empty_returns_zero() {
    let cache = ResultCache::new(256);
    let evicted = cache.invalidate(None);
    assert_eq!(evicted, 0);
}

#[test]
fn cache_multi_tool_source_invalidation() {
    let cache = ResultCache::new(256);
    cache.put("evaluate_impact", "o/r", "k1", json!(1));
    cache.put("find_references", "o/r", "k2", json!(2));
    cache.put("evaluate_impact", "o/other", "k3", json!(3));
    let evicted = cache.invalidate(Some("o/r"));
    assert_eq!(evicted, 2);
    // o/other survives
    assert_eq!(
        cache.get("evaluate_impact", "o/other", "k3"),
        Some(json!(3))
    );
    // o/r entries gone
    assert!(cache.get("evaluate_impact", "o/r", "k1").is_none());
    assert!(cache.get("find_references", "o/r", "k2").is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Result Cache: LRU Eviction
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn cache_evicts_oldest_when_full() {
    let max = 4;
    let cache = ResultCache::new(max);
    for i in 0..max {
        cache.put("tool", "o/r", &format!("k{i}"), json!(i));
    }
    // Cache is full. Add one more.
    cache.put("tool", "o/r", "kextra", json!("extra"));
    let s = cache.stats();
    assert_eq!(s.cached_entries, max);
    // Oldest (k0) should have been evicted.
    assert!(cache.get("tool", "o/r", "k0").is_none());
    // Newest should be present.
    assert_eq!(cache.get("tool", "o/r", "kextra"), Some(json!("extra")));
}

#[test]
fn cache_lru_refresh_on_get_delays_eviction() {
    let max = 4;
    let cache = ResultCache::new(max);
    // Fill with k0..k3
    for i in 0..max {
        cache.put("tool", "o/r", &format!("k{i}"), json!(i));
    }
    // Access k0 to refresh its LRU position.
    let _ = cache.get("tool", "o/r", "k0");
    // Add two more entries to push over capacity twice.
    cache.put("tool", "o/r", "knew1", json!("new1"));
    cache.put("tool", "o/r", "knew2", json!("new2"));
    // k0 should survive (was recently accessed).
    assert!(cache.get("tool", "o/r", "k0").is_some());
    // k1 should have been evicted first (oldest after k0 was refreshed).
    assert!(cache.get("tool", "o/r", "k1").is_none());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Result Cache: Session Stats Integration
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn session_stats_includes_result_cache() {
    let cache = ResultCache::new(256);
    cache.put("tool", "o/r", "k1", json!(1));
    let _ = cache.get("tool", "o/r", "k1"); // hit
    let _ = cache.get("tool", "o/r", "miss"); // miss
    let s = cache.stats();
    assert_eq!(s.total_hits, 1);
    assert_eq!(s.total_misses, 1);
    assert!((s.hit_rate - 0.5).abs() < f64::EPSILON);
    assert_eq!(s.cached_entries, 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Rate Limiting: Factory Function
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn rate_limit_returns_none_when_limit_zero() {
    assert!(make_rate_limiter(Some(0)).is_none());
}

#[test]
fn rate_limit_returns_none_when_unset() {
    assert!(make_rate_limiter(None).is_none());
}

#[test]
fn rate_limit_returns_none_for_invalid_value() {
    // The Rust API takes Option<i64>, so "invalid" non-numeric doesn't apply directly.
    // We verify that None (the equivalent of a parse failure) returns None.
    assert!(make_rate_limiter(None).is_none());
}

#[test]
fn rate_limit_returns_none_for_negative_value() {
    assert!(make_rate_limiter(Some(-5)).is_none());
}

#[test]
fn rate_limit_returns_limiter_when_positive() {
    assert!(make_rate_limiter(Some(10)).is_some());
}

#[test]
fn rate_limit_returns_limiter_for_limit_one() {
    assert!(make_rate_limiter(Some(1)).is_some());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Rate Limiting: Sliding-Window Bucket Logic
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn rate_limit_under_limit_all_allowed() {
    let limiter = RateLimiter::new(5, 60);
    let results: Vec<bool> = (0..5).map(|_| limiter.try_acquire()).collect();
    assert_eq!(results, vec![true, true, true, true, true]);
}

#[test]
fn rate_limit_over_limit_rejected() {
    let limiter = RateLimiter::new(3, 60);
    let results: Vec<bool> = (0..5).map(|_| limiter.try_acquire()).collect();
    assert_eq!(results, vec![true, true, true, false, false]);
}

#[test]
fn rate_limit_expired_entries_evicted() {
    let limiter = RateLimiter::new(2, 60);
    // Fill the bucket.
    assert!(limiter.try_acquire());
    assert!(limiter.try_acquire());
    assert!(!limiter.try_acquire()); // over limit

    // Advance time past the 60-second window.
    limiter.advance_time(Duration::from_secs(65));

    // Old entries should be evicted; new request allowed.
    assert!(limiter.try_acquire());
}

#[test]
fn rate_limit_one_allows_first_denies_second() {
    let limiter = RateLimiter::new(1, 60);
    let results: Vec<bool> = (0..2).map(|_| limiter.try_acquire()).collect();
    assert_eq!(results, vec![true, false]);
}
