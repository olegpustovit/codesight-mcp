//! Integration tests: agent v1 semantic.

use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, MutexGuard};

use codesight_mcp::config::AppConfig;
use codesight_mcp::semantic::{self, EmbeddingStore};
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

// ── Cosine similarity (pure math, no ignore) ────────────────────────────────
/// Identical unit vectors must yield cosine similarity 1.0.
#[test]
fn cosine_similarity_identical_vectors() {
    let v = vec![1.0_f64, 0.0, 0.0];
    let sim = cosine_similarity(&v, &v);
    assert!(
        (sim - 1.0).abs() < 1e-9,
        "identical vectors should have similarity 1.0, got {sim}"
    );
}
/// Orthogonal vectors must yield cosine similarity 0.0.
#[test]
fn cosine_similarity_orthogonal_vectors() {
    let sim = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
    assert!(
        sim.abs() < 1e-9,
        "orthogonal vectors should have similarity 0.0, got {sim}"
    );
}
/// A zero vector paired with any vector must return 0.0 (not NaN).
#[test]
fn cosine_similarity_zero_vector_returns_zero() {
    let sim = cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]);
    assert_eq!(sim, 0.0, "zero vector should yield 0.0");
}
/// Mismatched-length vectors must return 0.0 (graceful degradation).
#[test]
fn cosine_similarity_mismatched_lengths_returns_zero() {
    let sim = cosine_similarity(&[1.0, 2.0], &[1.0]);
    assert_eq!(sim, 0.0, "mismatched lengths should yield 0.0");
}
/// Opposite vectors must yield cosine similarity -1.0.
#[test]
fn cosine_similarity_opposite_vectors() {
    let sim = cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]);
    assert!(
        (sim - (-1.0)).abs() < 1e-9,
        "opposite vectors should have similarity -1.0, got {sim}"
    );
}

// ── Pure-math helper (standalone, no crate dependency) ───────────────────────

/// Cosine similarity matching the reference `_cosine_similarity` behavior:
/// - zero-vector returns 0.0
/// - mismatched lengths returns 0.0
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

static EMBEDDING_ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_embedding_env() -> MutexGuard<'static, ()> {
    EMBEDDING_ENV_LOCK
        .lock()
        .expect("embedding env mutex poisoned")
}

/// Clear all embedding-related env vars so tests start from a clean slate.
///
/// SAFETY: env var mutation is not thread-safe. These tests must not run
/// concurrently with other tests that read these vars.
fn clear_embedding_env() {
    for key in &[
        "CODESIGHT_EMBED_MODEL",
        "GOOGLE_API_KEY",
        "GOOGLE_EMBED_MODEL",
        "OPENAI_API_KEY",
        "OPENAI_EMBED_MODEL",
    ] {
        // SAFETY: test environment only; no other threads reading these vars simultaneously.
        unsafe { std::env::remove_var(key) };
    }
}

/// SAFETY: see `clear_embedding_env`.
fn set_env(key: &str, val: &str) {
    unsafe { std::env::set_var(key, val) };
}

fn make_svc() -> (tempfile::TempDir, CodeService) {
    let dir = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(dir.path().to_path_buf())).unwrap();
    (dir, svc)
}

fn index_semantic_repo(svc: &CodeService, dir: &std::path::Path) -> String {
    let repo = dir.join("semantic-repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("pool.py"),
        r#"
def connection_pool():
    """Manages database connections and pooling."""
    pass

def render_html(template):
    """Renders an HTML template to string."""
    pass

def parse_config(path):
    """Parses a YAML configuration file."""
    pass
"#,
    )
    .unwrap();

    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": repo.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

// ── Provider detection ───────────────────────────────────────────────────────
/// When no embedding env vars are set, detect_provider must return None.
#[test]
fn detect_provider_none_when_nothing_set() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    let result = semantic::detect_embedding_provider();
    assert!(
        result.is_none(),
        "expected None when no env vars set, got {:?}",
        result
    );
}
/// CODESIGHT_EMBED_MODEL set must yield ("sentence_transformers", model).
#[test]
fn detect_provider_sentence_transformers() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "all-MiniLM-L6-v2");
    let result = semantic::detect_embedding_provider();
    clear_embedding_env();
    let prov = result.expect("expected Some provider");
    assert_eq!(prov.provider, "sentence_transformers");
    assert_eq!(prov.model, "all-MiniLM-L6-v2");
}
/// GOOGLE_API_KEY + GOOGLE_EMBED_MODEL set must yield ("gemini", model).
#[test]
fn detect_provider_gemini() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("GOOGLE_API_KEY", "fake-key");
    set_env("GOOGLE_EMBED_MODEL", "embedding-001");
    let result = semantic::detect_embedding_provider();
    clear_embedding_env();
    let prov = result.expect("expected Some provider");
    assert_eq!(prov.provider, "gemini");
    assert_eq!(prov.model, "embedding-001");
}
/// OPENAI_API_KEY + OPENAI_EMBED_MODEL set must yield ("openai", model).
#[test]
fn detect_provider_openai() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("OPENAI_API_KEY", "sk-fake");
    set_env("OPENAI_EMBED_MODEL", "text-embedding-3-small");
    let result = semantic::detect_embedding_provider();
    clear_embedding_env();
    let prov = result.expect("expected Some provider");
    assert_eq!(prov.provider, "openai");
    assert_eq!(prov.model, "text-embedding-3-small");
}
/// sentence-transformers takes priority when multiple providers are set.
#[test]
fn detect_provider_sentence_transformers_wins_priority() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "all-MiniLM-L6-v2");
    set_env("GOOGLE_API_KEY", "fake-key");
    set_env("GOOGLE_EMBED_MODEL", "embedding-001");
    let result = semantic::detect_embedding_provider();
    clear_embedding_env();
    let prov = result.expect("expected Some provider");
    assert_eq!(
        prov.provider, "sentence_transformers",
        "sentence_transformers should win priority"
    );
}
/// OPENAI_API_KEY alone (no OPENAI_EMBED_MODEL) must NOT activate OpenAI embeddings.
#[test]
fn detect_provider_openai_requires_embed_model() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("OPENAI_API_KEY", "sk-fake");
    // Deliberately NOT setting OPENAI_EMBED_MODEL
    let result = semantic::detect_embedding_provider();
    clear_embedding_env();
    assert!(
        result.is_none(),
        "OPENAI_API_KEY alone should not activate provider, got {:?}",
        result
    );
}

// ── No-provider error handling ───────────────────────────────────────────────
/// semantic=true with no provider configured must return a structured error.
#[test]
fn semantic_true_no_provider_returns_error() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    let result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": "database pool",
                "domain": "code",
                "semantic": true,
            }),
        )
        .unwrap();

    assert_eq!(result["ok"].as_bool(), Some(false), "expected error");
    let err = &result["error"];
    assert_eq!(
        err["code"].as_str(),
        Some("no_embedding_provider"),
        "error code should be no_embedding_provider"
    );
    assert!(
        err["message"].as_str().is_some(),
        "error should have a message"
    );
}
/// embed_repo with no provider configured must return a structured error.
#[test]
fn embed_repo_no_provider_returns_error() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    let result = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();

    assert_eq!(result["ok"].as_bool(), Some(false), "expected error");
    let err = &result["error"];
    assert_eq!(
        err["code"].as_str(),
        Some("no_embedding_provider"),
        "error code should be no_embedding_provider"
    );
    assert!(
        err["message"].as_str().is_some(),
        "error should have a message"
    );
}

// ── Hybrid search ────────────────────────────────────────────────────────────
/// Hybrid search must return "connection_pool" for query "database pool" even
/// when BM25 alone would miss it — the semantic component bridges the gap.
#[test]
fn semantic_hybrid_surfaces_missed_bm25_symbol() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "tfidf-local");

    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    // Embed the repo (TF-IDF)
    let embed_result = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(
        embed_result["ok"].as_bool().unwrap(),
        "embed failed: {embed_result}"
    );

    // Search with semantic=true
    let result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": "database pool",
                "domain": "code",
                "semantic": true,
            }),
        )
        .unwrap();

    assert!(result["ok"].as_bool().unwrap(), "search failed: {result}");
    let items = result["data"]["items"].as_array().unwrap();
    assert!(!items.is_empty(), "expected at least one result");

    // connection_pool should appear (its docstring mentions "database connections")
    let names: Vec<&str> = items
        .iter()
        .filter_map(|i| i["ref"]["title"].as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("connection_pool")),
        "expected connection_pool in results, got: {names:?}"
    );

    // Check search_mode in _meta
    let search_mode = result["_meta"]["search_mode"].as_str();
    assert_eq!(search_mode, Some("hybrid"), "expected hybrid search mode");

    clear_embedding_env();
}

// ── Semantic weight zero ─────────────────────────────────────────────────────
/// semantic_weight=0.0 must return identical ranking to pure BM25.
#[test]
fn semantic_weight_zero_equals_pure_bm25() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "tfidf-local");

    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    // Embed
    let embed_result = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(embed_result["ok"].as_bool().unwrap());

    // Pure BM25 search
    let bm25_result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": "connection_pool",
                "domain": "code",
            }),
        )
        .unwrap();
    assert!(bm25_result["ok"].as_bool().unwrap());

    // Semantic with weight=0.0 (should be equivalent to BM25)
    let sem_result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": "connection_pool",
                "domain": "code",
                "semantic": true,
                "semantic_weight": 0.0,
            }),
        )
        .unwrap();
    assert!(sem_result["ok"].as_bool().unwrap());

    // Both should rank connection_pool first
    let bm25_first = bm25_result["data"]["items"][0]["ref"]["title"]
        .as_str()
        .unwrap_or("");
    let sem_first = sem_result["data"]["items"][0]["ref"]["title"]
        .as_str()
        .unwrap_or("");

    assert!(
        bm25_first.contains("connection_pool"),
        "BM25 should rank connection_pool first, got: {bm25_first}"
    );
    assert!(
        sem_first.contains("connection_pool"),
        "semantic_weight=0.0 should rank connection_pool first, got: {sem_first}"
    );

    clear_embedding_env();
}

// ── Semantic-only mode ───────────────────────────────────────────────────────
/// semantic_only=true must surface symbols with no BM25 token overlap.
#[test]
fn semantic_only_skips_bm25() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "tfidf-local");

    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    // Embed
    let embed_result = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(embed_result["ok"].as_bool().unwrap());

    // semantic_only=true with a query that has overlap with "connection" / "database"
    // The TF-IDF vectors should still find connection_pool due to vocabulary overlap
    let result = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": source_id,
                "query": "database connections pooling",
                "domain": "code",
                "semantic": true,
                "semantic_only": true,
            }),
        )
        .unwrap();

    assert!(result["ok"].as_bool().unwrap(), "search failed: {result}");

    let search_mode = result["_meta"]["search_mode"].as_str();
    assert_eq!(
        search_mode,
        Some("semantic_only"),
        "expected semantic_only search mode"
    );

    let items = result["data"]["items"].as_array().unwrap();
    assert!(
        !items.is_empty(),
        "expected at least one result in semantic_only mode"
    );

    // connection_pool should rank first (closest embedding to "database connections pooling")
    let first_name = items[0]["ref"]["title"].as_str().unwrap_or("");
    assert!(
        first_name.contains("connection_pool"),
        "expected connection_pool first in semantic_only mode, got: {first_name}"
    );

    clear_embedding_env();
}

// ── Embedding store persistence ──────────────────────────────────────────────
/// Embeddings must survive a simulated process restart (new store instance).
#[test]
fn embedding_store_persist_and_retrieve() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("emb.json");

    // Create store, insert embedding
    {
        let mut store = EmbeddingStore::open(&path);
        let mut items = HashMap::new();
        items.insert("s1".to_string(), vec![0.1, 0.2, 0.3, 0.4]);
        store.set_many(items);
        store.set_dimension(4, "test-model");
    }

    // New instance from same path — simulates restart
    {
        let store = EmbeddingStore::open(&path);
        let vec = store.get("s1").expect("s1 should be persisted");
        assert_eq!(vec.len(), 4);
        for (i, &expected) in [0.1, 0.2, 0.3, 0.4].iter().enumerate() {
            assert!(
                (vec[i] - expected).abs() < 1e-5,
                "element {i}: expected {expected}, got {}",
                vec[i]
            );
        }
        assert_eq!(store.get_dimension(), Some(4));
    }
}
/// delete_many must remove specified embeddings and leave others intact.
#[test]
fn embedding_store_delete_many() {
    let mut store = EmbeddingStore::new();
    let mut items = HashMap::new();
    items.insert("s1".to_string(), vec![1.0, 0.0]);
    items.insert("s2".to_string(), vec![0.0, 1.0]);
    store.set_many(items);
    assert_eq!(store.count(), 2);

    store.delete_many(&["s1"]);
    assert_eq!(store.count(), 1);
    assert!(store.get("s1").is_none(), "s1 should be deleted");
    assert!(store.get("s2").is_some(), "s2 should remain");
}

// ── embed_repo caching ───────────────────────────────────────────────────────
/// embed_repo must not recompute embeddings that are already stored.
#[test]
fn embed_repo_caches_and_skips_on_second_call() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "tfidf-local");

    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    // First call: should embed symbols
    let r1 = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(r1["ok"].as_bool().unwrap());
    let first_embedded = r1["data"]["symbols_embedded"].as_u64().unwrap();
    assert!(first_embedded > 0, "first call should embed symbols");
    assert_eq!(r1["data"]["cached"].as_bool(), Some(false));

    // Second call: should skip (cached)
    let r2 = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(r2["ok"].as_bool().unwrap());
    assert_eq!(
        r2["data"]["symbols_embedded"].as_u64(),
        Some(0),
        "second call should embed 0 symbols"
    );
    assert_eq!(r2["data"]["cached"].as_bool(), Some(true));

    clear_embedding_env();
}
/// force=true must recompute even when embeddings already exist.
#[test]
fn embed_repo_force_recomputes() {
    let _env = lock_embedding_env();
    clear_embedding_env();
    set_env("CODESIGHT_EMBED_MODEL", "tfidf-local");

    let (dir, svc) = make_svc();
    let source_id = index_semantic_repo(&svc, dir.path());

    // First call: embed
    let r1 = svc
        .dispatch("embed_repo", json!({ "source_id": source_id }))
        .unwrap();
    assert!(r1["ok"].as_bool().unwrap());
    let first_count = r1["data"]["symbols_embedded"].as_u64().unwrap();
    assert!(first_count > 0);

    // Second call with force=true: should recompute
    let r2 = svc
        .dispatch(
            "embed_repo",
            json!({ "source_id": source_id, "force": true }),
        )
        .unwrap();
    assert!(r2["ok"].as_bool().unwrap());
    let second_count = r2["data"]["symbols_embedded"].as_u64().unwrap();
    assert_eq!(
        second_count, first_count,
        "force=true should re-embed all symbols"
    );

    clear_embedding_env();
}
