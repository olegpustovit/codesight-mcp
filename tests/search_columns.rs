//! Integration tests: search columns.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_storage::storage::IndexStore;
use serde_json::json;
use tempfile::tempdir;

// ── helpers ──────────────────────────────────────────────────────────────

fn setup_with_metadata(metadata: serde_json::Value) -> (CodeService, String) {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("model.py"), "x = 1\n").unwrap();

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();

    // Store the context_metadata in the index's meta table
    let repos_dir = tmp.path().join("idx").join("repos");
    for entry in fs::read_dir(&repos_dir).unwrap() {
        let entry = entry.unwrap();
        let db_path = entry.path().join("index.sqlite3");
        if db_path.exists() {
            let store = IndexStore::open(db_path).unwrap();
            store
                .set_meta(
                    "context_metadata",
                    &serde_json::to_string(&metadata).unwrap(),
                )
                .unwrap();
        }
    }

    // We need to keep tmp alive, leak it
    std::mem::forget(tmp);
    (svc, sid)
}

#[test]
fn case_search_columns_exact_match() {
    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {
            "orders": {"order_id": "Primary key", "status": "Order status"},
        },
    }));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "order_id",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data["result_count"].as_u64().unwrap() >= 1,
        "expected results: {data:?}"
    );
    let top = &data["results"][0];
    assert_eq!(top["column"].as_str().unwrap(), "order_id");
    // Exact name match scores 30
    assert_eq!(
        top["score"].as_i64().unwrap(),
        30,
        "exact match should score 30: {top:?}"
    );
}

#[test]
fn case_search_columns_partial_match() {
    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {
            "orders": {"order_id": "Primary key", "customer_id": "FK to customers"},
        },
    }));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "order",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let results = r["data"]["results"].as_array().unwrap();
    let matches: Vec<_> = results
        .iter()
        .filter(|r| r["column"] == "order_id")
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected 1 match on order_id: {results:?}"
    );
    // "order" in "order_id" -> 15
    assert_eq!(
        matches[0]["score"].as_i64().unwrap(),
        15,
        "partial match should score 15: {:?}",
        matches[0]
    );
}

#[test]
fn case_search_columns_model_pattern_filter() {
    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {
            "fact_orders": {"amount": "Order total"},
            "dim_customers": {"name": "Customer name"},
            "fact_payments": {"method": "Payment method"},
        },
    }));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "name method amount",
                "model_pattern": "fact_*",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let results = r["data"]["results"].as_array().unwrap();
    let models: std::collections::HashSet<&str> =
        results.iter().filter_map(|r| r["model"].as_str()).collect();
    assert!(
        !models.contains("dim_customers"),
        "dim_customers should be filtered out: {models:?}"
    );
    // At least one fact_ model should appear
    assert!(
        models.iter().all(|m| m.starts_with("fact_")),
        "all models should match fact_*: {models:?}"
    );
}

#[test]
fn case_search_columns_multi_provider() {
    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {
            "orders": {"order_id": "Primary key"},
        },
        "sqlmesh_columns": {
            "payments": {"payment_id": "Primary key"},
        },
    }));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "id",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data["result_count"].as_u64().unwrap() >= 2,
        "expected >= 2 results: {data:?}"
    );
    let sources = data["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 2, "expected 2 sources: {sources:?}");
    // Every result should have a "source" field
    for result in data["results"].as_array().unwrap() {
        assert!(
            result.get("source").is_some(),
            "missing source field: {result:?}"
        );
        let source = result["source"].as_str().unwrap();
        assert!(
            source == "dbt" || source == "sqlmesh",
            "unexpected source: {source}"
        );
    }
}

#[test]
fn case_search_columns_no_metadata() {
    let (svc, sid) = setup_with_metadata(json!({}));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "anything",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
    let error_msg = r["error"]["message"].as_str().unwrap_or("").to_lowercase();
    assert!(
        error_msg.contains("column metadata"),
        "expected error about column metadata: {error_msg}"
    );
}

#[test]
fn case_search_columns_max_results_cap() {
    // Create enough columns to exceed the cap
    let mut columns = serde_json::Map::new();
    for i in 0..50 {
        columns.insert(format!("col_{i}"), json!(format!("description {i}")));
    }

    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {
            "big_model": columns,
        },
    }));

    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "description",
                "max_results": 5,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data["result_count"].as_u64().unwrap() <= 5,
        "expected <= 5 results: {data:?}"
    );
    assert!(
        data["results"].as_array().unwrap().len() <= 5,
        "expected <= 5 result items: {data:?}"
    );
}

#[test]
fn case_collect_all_columns_multiple_keys() {
    // This tests the internal _collect_all_columns helper.
    // We test it indirectly through the search_columns tool by verifying
    // that both dbt and sqlmesh columns are found.
    let (svc, sid) = setup_with_metadata(json!({
        "dbt_columns": {"m1": {"c1": "d1"}},
        "sqlmesh_columns": {"m2": {"c2": "d2"}},
        "unrelated_key": "ignored",
    }));

    // Search for something that matches both providers
    let r = svc
        .dispatch(
            "search_columns",
            json!({
                "source_id": sid,
                "query": "c1 c2 d1 d2",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    let sources = data["sources"].as_array().unwrap();
    let source_names: std::collections::HashSet<&str> =
        sources.iter().filter_map(|s| s.as_str()).collect();
    assert!(
        source_names.contains("dbt"),
        "expected dbt source: {source_names:?}"
    );
    assert!(
        source_names.contains("sqlmesh"),
        "expected sqlmesh source: {source_names:?}"
    );
}
