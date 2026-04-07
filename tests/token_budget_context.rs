//! Integration tests: token budget context.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

const SMALL_ENGINE: &str = concat!(
    "class Engine:\n",
    "    \"\"\"Core engine.\"\"\"\n",
    "    def run(self):\n",
    "        pass\n\n",
    "    def stop(self):\n",
    "        pass\n",
);

const SMALL_UTILS: &str =
    "def format_date(d):\n    return str(d)\n\ndef parse_date(s):\n    return s\n";

const SMALL_MAIN: &str = "from engine import Engine\nfrom utils import format_date\n\ndef main():\n    e = Engine()\n    e.run()\n";

#[test]
fn token_budget_context_suite() {
    run_all();
}

fn run_all() {
    case_bundle_budget_none_full_source();
    case_bundle_compact_multi_empty_source();
    case_bundle_compact_single_keeps_signature();
    case_bundle_tiny_budget_report_or_ok();
    case_bundle_no_report_by_default();
    case_bundle_report_fields();
    case_bundle_used_tokens_le_budget();
    case_bundle_invalid_strategy_error();
    case_bundle_flat_envelope();
    case_ranked_context_items();
    case_ranked_total_tokens_under_budget();
    case_ranked_items_have_source_and_scores();
    case_ranked_bm25_strategy();
    case_ranked_zero_budget_error();
    case_ranked_include_kinds_class();
    case_ranked_unknown_repo();
    case_ranked_query_too_long();
    case_ranked_meta_envelope();
}

fn write_small_repo(root: &std::path::Path) {
    fs::write(root.join("engine.py"), SMALL_ENGINE).unwrap();
    fs::write(root.join("utils.py"), SMALL_UTILS).unwrap();
    fs::write(root.join("main.py"), SMALL_MAIN).unwrap();
}

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    ing["data"]["source_id"].as_str().unwrap().to_string()
}

fn engine_class_id(svc: &CodeService, sid: &str) -> String {
    let s = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "Engine",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "kind": "class",
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(s["ok"].as_bool().unwrap(), "{s:?}");
    s["data"]["items"][0]["ref"]["stable_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn format_date_id(svc: &CodeService, sid: &str) -> String {
    let s = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "format_date",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    assert!(s["ok"].as_bool().unwrap(), "{s:?}");
    s["data"]["items"][0]["ref"]["stable_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn case_bundle_budget_none_full_source() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": eid }],
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    assert!(!out["data"]["source"].as_str().unwrap().is_empty());
}

fn case_bundle_compact_multi_empty_source() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let fid = format_date_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [
                    { "kind": "symbol", "stable_id": eid },
                    { "kind": "symbol", "stable_id": fid }
                ],
                "token_budget": 10000,
                "budget_strategy": "compact",
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    for sym in out["data"]["symbols"].as_array().unwrap() {
        assert!(sym["source"].as_str().unwrap().is_empty());
    }
}

fn case_bundle_compact_single_keeps_signature() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": eid }],
                "token_budget": 10000,
                "budget_strategy": "compact",
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    let sig = out["data"]["signature"].as_str().unwrap();
    assert!(!sig.is_empty(), "{sig:?}");
}

fn case_bundle_tiny_budget_report_or_ok() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let fid = format_date_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [
                    { "kind": "symbol", "stable_id": eid },
                    { "kind": "symbol", "stable_id": fid }
                ],
                "token_budget": 1,
                "budget_strategy": "most_relevant",
                "include_budget_report": true,
            }),
        )
        .unwrap();
    let ok = out["ok"].as_bool().unwrap();
    let has_report = out["data"].get("budget_report").is_some();
    assert!(
        !ok || has_report,
        "expected budget_report on success for tiny budget: {out:?}"
    );
    if ok {
        assert!(has_report);
    }
}

fn case_bundle_no_report_by_default() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": eid }],
                "max_tokens": 10000,
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    assert!(
        !out["data"]
            .as_object()
            .unwrap()
            .contains_key("budget_report")
    );
}

fn case_bundle_report_fields() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let fid = format_date_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [
                    { "kind": "symbol", "stable_id": eid },
                    { "kind": "symbol", "stable_id": fid }
                ],
                "token_budget": 10000,
                "include_budget_report": true,
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    let br = out["data"]["budget_report"].as_object().unwrap();
    for k in [
        "budget_tokens",
        "used_tokens",
        "included_symbols",
        "excluded_symbols",
        "strategy",
    ] {
        assert!(br.contains_key(k), "missing {k}");
    }
}

fn case_bundle_used_tokens_le_budget() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let fid = format_date_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [
                    { "kind": "symbol", "stable_id": eid },
                    { "kind": "symbol", "stable_id": fid }
                ],
                "token_budget": 200,
                "include_budget_report": true,
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    let br = out["data"]["budget_report"].as_object().unwrap();
    let used = br["used_tokens"].as_u64().unwrap();
    let cap = br["budget_tokens"].as_u64().unwrap();
    assert!(used <= cap);
}

fn case_bundle_invalid_strategy_error() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let eid = engine_class_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": eid }],
                "token_budget": 1000,
                "budget_strategy": "magic",
            }),
        )
        .unwrap();
    assert!(!out["ok"].as_bool().unwrap(), "{out:?}");
    let detail = out["error"]["detail"].to_string();
    assert!(detail.contains("budget_strategy"), "detail={detail}");
}

fn case_bundle_flat_envelope() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let fid = format_date_id(&svc, &sid);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "context_bundle",
                "refs": [{ "kind": "symbol", "stable_id": fid }],
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    let d = out["data"].as_object().unwrap();
    assert!(d.contains_key("symbol_id"));
    assert!(d.contains_key("source"));
    assert!(!d.contains_key("budget_report"));
}

fn case_ranked_context_items() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "engine run",
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    assert!(out["data"]["context_items"].is_array());
}

fn case_ranked_total_tokens_under_budget() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "engine",
                "token_budget": 100,
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    assert!(out["data"]["total_tokens"].as_u64().unwrap() <= 100);
}

fn case_ranked_items_have_source_and_scores() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "Engine",
                "token_budget": 4000,
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    for item in out["data"]["context_items"].as_array().unwrap() {
        assert!(item.get("source").is_some());
        assert!(item.get("relevance_score").is_some());
        assert!(item.get("centrality_score").is_some());
        assert!(item.get("combined_score").is_some());
        assert!(item.get("tokens").is_some());
    }
}

fn case_ranked_bm25_strategy() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "format date",
                "strategy": "bm25",
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    assert!(out["data"]["context_items"].is_array());
}

fn case_ranked_zero_budget_error() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "engine",
                "token_budget": 0,
            }),
        )
        .unwrap();
    assert!(!out["ok"].as_bool().unwrap(), "{out:?}");
}

fn case_ranked_include_kinds_class() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "engine",
                "token_budget": 4000,
                "include_kinds": ["class"],
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    for item in out["data"]["context_items"].as_array().unwrap() {
        let sid_sym = item["symbol_id"].as_str().unwrap();
        assert!(
            sid_sym.contains("#class"),
            "expected class symbol, got {sid_sym}"
        );
    }
}

fn case_ranked_unknown_repo() {
    let workspace = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": "no_such_repo_token_budget",
                "mode": "ranked",
                "query": "engine",
            }),
        )
        .unwrap();
    assert!(!out["ok"].as_bool().unwrap(), "{out:?}");
}

fn case_ranked_query_too_long() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let q = "x".repeat(501);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": q,
            }),
        )
        .unwrap();
    assert!(!out["ok"].as_bool().unwrap(), "{out:?}");
}

fn case_ranked_meta_envelope() {
    let workspace = tempdir().unwrap();
    let root = workspace.path();
    write_small_repo(root);
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, root);
    let out = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "ranked",
                "query": "Engine",
            }),
        )
        .unwrap();
    assert!(out["ok"].as_bool().unwrap(), "{out:?}");
    let meta = out["_meta"].as_object().unwrap();
    assert!(meta.contains_key("timing_ms"));
    assert!(meta.contains_key("tokens_saved"));
}
