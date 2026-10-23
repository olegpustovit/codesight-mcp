use std::fs;

use serde_json::json;
use symslice_engine::config::AppConfig;
use symslice_engine::service::CodeService;
use tempfile::tempdir;

fn ingest_code(svc: &CodeService, root: &std::path::Path) -> String {
    let response = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");
    response["data"]["source_id"].as_str().unwrap().to_string()
}

// ROADMAP P1.1: `search_catalog` charges against an `rg -n` equivalent output
// (path:line:snippet per returned hit) — not the sum of file sizes of every
// matched file. The huge unrelated blob in the same repo must not inflate the
// baseline, and a single-hit response must fit within a loose rg-row budget.
#[test]
fn search_catalog_savings_use_rg_equivalent_not_repo_bytes() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    fs::write(src.join("huge_blob.py"), "x".repeat(400_000)).unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let sid = ingest_code(&svc, ws.path());

    let response = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");

    // A single `def hello():` hit should cost at most a few rg rows' worth of
    // bytes. The 400KB unrelated blob must not show up in the baseline.
    let naive_tokens = response["_meta"]["naive_tokens"].as_u64().unwrap();
    assert!(
        naive_tokens < 30,
        "single-hit rg-equivalent baseline should be bounded (~path:line:snippet), got {naive_tokens}: {response:?}"
    );
    let estimate_method = response["_meta"]["estimate_method"].as_str().unwrap_or("");
    assert_eq!(
        estimate_method, "rg_equivalent_output_bytes",
        "{response:?}"
    );
}

// `output_mode=count` doesn't return items, so the rg-equivalent baseline is
// zero by construction; savings are undefined for a metadata-only response.
#[test]
fn search_catalog_count_mode_emits_no_inflated_baseline() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();
    fs::write(src.join("huge_blob.py"), "y".repeat(400_000)).unwrap();

    let svc =
        symslice_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx")))
            .unwrap();
    let sid = ingest_code(&svc, ws.path());

    let response = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "hello",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "output_mode": "count",
                "limit": 5,
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    assert!(response["ok"].as_bool().unwrap(), "{response:?}");

    let naive_tokens = response["_meta"]["naive_tokens"].as_u64().unwrap();
    assert!(
        naive_tokens < 30,
        "count mode must not charge against the whole-repo / huge-blob file; got {naive_tokens}: {response:?}"
    );
}
