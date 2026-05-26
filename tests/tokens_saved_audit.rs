//! ROADMAP Enforcement §3 — token-savings audit. Runs a fixed set of tool
//! calls against a known fixture and asserts each per-call `tokens_saved`
//! stays within the ROADMAP's honesty contract:
//!
//!   * `read_content` of a symbol inside a larger file: `tokens_saved` is
//!     positive but bounded by `full_file_bytes / BYTES_PER_TOKEN`.
//!   * Overview tools (`repository_rollup`, `catalog_sources`,
//!     `manage_indexes action=status`) report `tokens_saved: null` — the
//!     field is omitted from `_meta`.
//!   * `search_catalog` baseline is an `rg -n`-shaped small number, never
//!     the sum-of-all-matched-files inflation we used to fabricate.

use std::fs;

use codesight_engine::config::AppConfig;
use serde_json::{Value, json};
use tempfile::tempdir;

fn build_fixture() -> (
    tempfile::TempDir,
    codesight_engine::service::CodeService,
    String,
) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("repo").join("src");
    fs::create_dir_all(&src).unwrap();
    // One small target symbol embedded in a file made artificially large
    // with a long comment block so a `read_content symbol` has a clear
    // budget advantage over a full-file read.
    let mut big = String::new();
    big.push_str("# padding\n".repeat(200).as_str());
    big.push_str("def target_symbol():\n    return 42\n");
    big.push_str("# trailing padding\n".repeat(200).as_str());
    fs::write(src.join("mod.py"), &big).unwrap();
    fs::write(src.join("unrelated.py"), "def other(): return 1\n").unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().join("repo").to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, sid)
}

fn tokens_saved(response: &Value) -> Option<u64> {
    response["_meta"]
        .get("tokens_saved")
        .and_then(Value::as_u64)
}

#[test]
fn read_content_symbol_savings_bounded_by_full_file() {
    let (_ws, svc, sid) = build_fixture();
    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "target_symbol",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 1,
            }),
        )
        .unwrap();
    let ref_id = search["data"]["items"][0]["ref"]["id"]
        .as_str()
        .expect("symbol hit");

    let read = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [{ "kind": "symbol", "stable_id": ref_id }],
                // `naive_tokens` lives in the verbose meta only.
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    let saved = tokens_saved(&read).expect("symbol read should carry tokens_saved");
    assert!(
        saved > 0,
        "read_content symbol expected positive savings: {read:?}"
    );
    let naive = read["_meta"]["naive_tokens"]
        .as_u64()
        .expect("verbose meta exposes naive_tokens");
    assert!(
        saved <= naive,
        "savings must not exceed naive tokens budget: saved={saved} naive={naive}"
    );
}

#[test]
fn overview_tools_omit_tokens_saved() {
    let (_ws, svc, sid) = build_fixture();
    let catalog = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(
        catalog["_meta"].get("tokens_saved").is_none(),
        "catalog_sources must omit tokens_saved: {catalog:?}"
    );

    let status = svc
        .dispatch(
            "manage_indexes",
            json!({ "action": "status", "source_id": sid }),
        )
        .unwrap();
    assert!(
        status["_meta"].get("tokens_saved").is_none(),
        "manage_indexes status must omit tokens_saved: {status:?}"
    );

    let rollup = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "repository_rollup" }),
        )
        .unwrap();
    assert!(
        rollup["_meta"].get("tokens_saved").is_none(),
        "repository_rollup must omit tokens_saved: {rollup:?}"
    );
}

#[test]
fn search_catalog_baseline_is_rg_equivalent_not_whole_file() {
    let (_ws, svc, sid) = build_fixture();
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "target_symbol",
                "query_mode": "keyword",
                "unit_kinds": ["symbol"],
                "limit": 5,
                "_meta_verbose": true,
            }),
        )
        .unwrap();
    let naive = r["_meta"]["naive_tokens"].as_u64().unwrap();
    // The fixture file is 400+ padding lines; sum-of-files would put
    // naive_tokens in the high hundreds or thousands. The rg-equivalent
    // for one symbol hit must be under a tight budget.
    assert!(
        naive < 40,
        "search_catalog baseline leaked the whole file: naive={naive}"
    );
    assert_eq!(
        r["_meta"]["estimate_method"].as_str(),
        Some("rg_equivalent_output_bytes")
    );
}
