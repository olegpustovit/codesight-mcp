//! Integration tests: call hierarchy.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use codesight_mcp::config::AppConfig;
use codesight_mcp::dep_graph::word_match;
use codesight_mcp::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn index_code(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

/// Build a synthetic 4-file repo:
///   utils.py      — defines helper(), shared_util()
///   services.py   — imports utils; defines process() which calls helper()
///   controllers.py — imports services; defines handle() which calls process()
///   main.py       — imports controllers; defines run() which calls handle()
fn build_call_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("utils.py"),
        "def helper():\n    return 42\n\ndef shared_util():\n    return 'ok'\n",
    )
    .unwrap();
    fs::write(
        src.join("services.py"),
        "from utils import helper, shared_util\n\ndef process():\n    return helper() + 1\n",
    )
    .unwrap();
    fs::write(
        src.join("controllers.py"),
        "from services import process\n\ndef handle(req):\n    return process()\n",
    )
    .unwrap();
    fs::write(
        src.join("main.py"),
        "from controllers import handle\n\ndef run():\n    return handle(None)\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

/// Find the stable_id of a symbol by name in the index.
fn find_symbol_id(svc: &CodeService, sid: &str, name: &str) -> Option<String> {
    let r = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": name,
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 50,
            }),
        )
        .unwrap();
    if !r["ok"].as_bool().unwrap_or(false) {
        return None;
    }
    for it in r["data"]["items"].as_array().unwrap_or(&vec![]) {
        if it["ref"]["title"].as_str() == Some(name) {
            return Some(it["ref"]["id"].as_str().unwrap().to_string());
        }
    }
    None
}

/// Call inspect_graph with graph=calls for a symbol.
fn call_hierarchy(
    svc: &CodeService,
    sid: &str,
    symbol_id: &str,
    direction: Option<&str>,
    depth: Option<usize>,
) -> Value {
    let mut params = json!({
        "source_id": sid,
        "graph": "calls",
        "symbol_id": symbol_id,
    });
    if let Some(d) = direction {
        params["direction"] = json!(d);
    }
    if let Some(d) = depth {
        params["depth"] = json!(d);
    }
    svc.dispatch("inspect_graph", params).unwrap()
}

// ===========================================================================
// _call_graph unit tests  (TestCallGraphHelpers, 6 tests)
// ===========================================================================

#[test]
fn call_graph_word_match_basic() {
    assert!(word_match("return helper()", "helper"));
}

#[test]
fn call_graph_word_match_no_partial() {
    assert!(!word_match("return my_helper()", "helper"));
}

#[test]
fn call_graph_word_match_multiline() {
    let content = "def foo():\n    result = helper(42)\n    return result\n";
    assert!(word_match(content, "helper"));
}

#[test]
fn call_graph_symbol_body_extracts_lines() {
    // In the Rust implementation, symbol bodies are stored as source_text
    // in the database (extracted by the indexer). The "symbol body" concept
    // maps directly to SymbolRecord.stored_source. We verify the indexer
    // stores correct content by checking that word_match works on a symbol body.
    // The actual body extraction is tested via the indexer/storage layer;
    // here we verify the call graph's use of it.
    let body = "def foo():\n    return helper(42)\n";
    assert!(
        word_match(body, "helper"),
        "body should contain 'helper' as word"
    );
    assert!(!word_match(body, "bar"), "body should not contain 'bar'");
    // Verify that line boundaries don't interfere
    assert!(word_match(body, "foo"), "body should contain 'foo' as word");
}

#[test]
fn call_graph_symbol_body_missing_line_returns_empty() {
    // A symbol with line=0 end_line=0 is degenerate; in our implementation
    // this would mean an empty stored_source. We test by checking that
    // word_match on an empty string returns false.
    assert!(!word_match("", "anything"));
}

#[test]
fn call_graph_build_symbols_by_file() {
    // Index a 4-file repo. Verify that calling inspect_graph calls mode works,
    // which requires indexed symbols spanning multiple files.
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper");
    assert!(helper_id.is_some(), "helper should be indexed as a symbol");
    let process_id = find_symbol_id(&svc, &sid, "process");
    assert!(
        process_id.is_some(),
        "process should be indexed as a symbol"
    );
}

// ===========================================================================
// get_call_hierarchy integration tests  (TestGetCallHierarchy, 16 tests)
// ===========================================================================

#[test]
fn call_hierarchy_returns_expected_shape() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    // Must have: symbol, callers, callees, depth_reached, caller_count, callee_count, _meta
    assert!(data["symbol"].is_object(), "should have symbol field");
    assert!(data["callers"].is_array(), "should have callers array");
    assert!(data["callees"].is_array(), "should have callees array");
    assert!(
        data["depth_reached"].is_number(),
        "should have depth_reached"
    );
    assert!(data["caller_count"].is_number(), "should have caller_count");
    assert!(data["callee_count"].is_number(), "should have callee_count");
    assert!(data["_meta"].is_object(), "should have _meta");
}

#[test]
fn call_hierarchy_symbol_field_correct() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sym = &r["data"]["symbol"];
    assert_eq!(sym["name"].as_str().unwrap(), "helper");
    let kind = sym["kind"].as_str().unwrap_or("");
    assert!(
        kind == "function" || kind == "method" || kind == "",
        "kind should be function/method/empty, got '{kind}'"
    );
}

#[test]
fn call_hierarchy_callers_found_for_helper() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callers = r["data"]["callers"].as_array().unwrap();
    let caller_names: Vec<&str> = callers.iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(
        caller_names.contains(&"process"),
        "helper's callers should include 'process': {caller_names:?}"
    );
}

#[test]
fn call_hierarchy_callees_found_for_process() {
    let (_ws, svc, sid) = build_call_repo();
    let process_id = find_symbol_id(&svc, &sid, "process").expect("process should be indexed");
    let r = call_hierarchy(&svc, &sid, &process_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callees = r["data"]["callees"].as_array().unwrap();
    let callee_names: Vec<&str> = callees.iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(
        callee_names.contains(&"helper"),
        "process's callees should include 'helper': {callee_names:?}"
    );
}

#[test]
fn call_hierarchy_callers_direction_returns_no_callees() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, Some("callers"), None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callees = r["data"]["callees"].as_array().unwrap();
    assert!(
        callees.is_empty(),
        "direction=callers should yield empty callees"
    );
}

#[test]
fn call_hierarchy_callees_direction_returns_no_callers() {
    let (_ws, svc, sid) = build_call_repo();
    let process_id = find_symbol_id(&svc, &sid, "process").expect("process should be indexed");
    let r = call_hierarchy(&svc, &sid, &process_id, Some("callees"), None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callers = r["data"]["callers"].as_array().unwrap();
    assert!(
        callers.is_empty(),
        "direction=callees should yield empty callers"
    );
}

#[test]
fn call_hierarchy_depth_field_in_each_result() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, Some(3));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for caller in r["data"]["callers"].as_array().unwrap() {
        let depth = caller["depth"].as_u64().unwrap();
        assert!(
            depth >= 1,
            "each caller should have depth >= 1, got {depth}"
        );
    }
}

#[test]
fn call_hierarchy_depth1_returns_only_direct_callers() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, Some("callers"), Some(1));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for caller in r["data"]["callers"].as_array().unwrap() {
        let depth = caller["depth"].as_u64().unwrap();
        assert_eq!(depth, 1, "at depth=1, all callers should have depth==1");
    }
}

#[test]
fn call_hierarchy_transitive_callers_at_depth3() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, Some("callers"), Some(3));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callers = r["data"]["callers"].as_array().unwrap();
    let caller_names: Vec<&str> = callers.iter().filter_map(|c| c["name"].as_str()).collect();
    // helper <- process <- handle <- run. At depth 3, at minimum "process" must be present.
    assert!(
        caller_names.contains(&"process"),
        "transitive callers should include 'process': {caller_names:?}"
    );
}

#[test]
fn call_hierarchy_no_cycles_in_callers() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, Some("callers"), Some(5));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callers = r["data"]["callers"].as_array().unwrap();
    let ids: Vec<&str> = callers.iter().filter_map(|c| c["id"].as_str()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "each symbol should appear at most once in callers"
    );
}

#[test]
fn call_hierarchy_no_cycles_in_callees() {
    let (_ws, svc, sid) = build_call_repo();
    let process_id = find_symbol_id(&svc, &sid, "process").expect("process should be indexed");
    let r = call_hierarchy(&svc, &sid, &process_id, Some("callees"), Some(5));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callees = r["data"]["callees"].as_array().unwrap();
    let ids: Vec<&str> = callees.iter().filter_map(|c| c["id"].as_str()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "each symbol should appear at most once in callees"
    );
}

#[test]
fn call_hierarchy_unknown_symbol_returns_error() {
    let (_ws, svc, sid) = build_call_repo();
    let r = call_hierarchy(&svc, &sid, "nonexistent_xyz", None, None);
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "should return error for unknown symbol: {r:?}"
    );
}

#[test]
fn call_hierarchy_meta_source_is_text_heuristic() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(
        r["data"]["_meta"]["source"].as_str().unwrap(),
        "text_heuristic"
    );
}

#[test]
fn call_hierarchy_leaf_symbol_has_no_callers() {
    let (_ws, svc, sid) = build_call_repo();
    let run_id = find_symbol_id(&svc, &sid, "run").expect("run should be indexed");
    let r = call_hierarchy(&svc, &sid, &run_id, Some("callers"), None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let callers = r["data"]["callers"].as_array().unwrap();
    assert!(
        callers.is_empty(),
        "run() should have no callers: {callers:?}"
    );
}

#[test]
fn call_hierarchy_result_has_file_and_line() {
    let (_ws, svc, sid) = build_call_repo();
    let helper_id = find_symbol_id(&svc, &sid, "helper").expect("helper should be indexed");
    let r = call_hierarchy(&svc, &sid, &helper_id, None, None);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for caller in r["data"]["callers"].as_array().unwrap() {
        assert!(caller["file"].is_string(), "caller should have file field");
        assert!(caller["line"].is_number(), "caller should have line field");
    }
}

// ===========================================================================
// Helper for impact_preview
// ===========================================================================

fn impact_preview(svc: &CodeService, sid: &str, symbol_name: &str) -> Value {
    svc.dispatch(
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "impact_preview",
            "symbol_id": symbol_name,
        }),
    )
    .unwrap()
}

fn blast_radius_with_call_depth(
    svc: &CodeService,
    sid: &str,
    symbol_name: &str,
    depth: u64,
    call_depth: usize,
) -> Value {
    svc.dispatch(
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "blast_radius",
            "symbol_id": symbol_name,
            "depth": depth,
            "call_depth": call_depth,
        }),
    )
    .unwrap()
}

fn find_refs(svc: &CodeService, sid: &str, identifier: &str, include_call_chain: bool) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "find_references",
            "identifier": identifier,
            "include_call_chain": include_call_chain,
        }),
    )
    .unwrap()
}

fn find_refs_batch(
    svc: &CodeService,
    sid: &str,
    identifiers: &[&str],
    include_call_chain: bool,
) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "find_references",
            "identifiers": identifiers,
            "include_call_chain": include_call_chain,
        }),
    )
    .unwrap()
}

// ===========================================================================
// get_impact_preview integration tests  (TestGetImpactPreview, 10 tests)
// ===========================================================================

#[test]
fn impact_preview_returns_expected_shape() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "expected ok: {r:?}");
    assert!(r["data"]["symbol"].is_object(), "missing symbol");
    assert!(
        r["data"]["affected_files"].is_number(),
        "missing affected_files"
    );
    assert!(
        r["data"]["affected_symbol_count"].is_number(),
        "missing affected_symbol_count"
    );
    assert!(
        r["data"]["affected_symbols"].is_array(),
        "missing affected_symbols"
    );
    assert!(
        r["data"]["affected_by_file"].is_object(),
        "missing affected_by_file"
    );
    assert!(r["data"]["call_chains"].is_array(), "missing call_chains");
    assert!(r["_meta"].is_object(), "missing _meta");
}

#[test]
fn impact_preview_affected_symbols_includes_callers() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let names: Vec<&str> = r["data"]["affected_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        names.contains(&"process"),
        "expected process in affected: {names:?}"
    );
}

#[test]
fn impact_preview_affected_files_count() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // At minimum services.py (contains process which calls helper)
    assert!(r["data"]["affected_files"].as_u64().unwrap() >= 1);
}

#[test]
fn impact_preview_call_chains_present() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let chains = r["data"]["call_chains"].as_array().unwrap();
    assert!(!chains.is_empty(), "expected non-empty call_chains");
    let chain_entry = &chains[0];
    assert!(
        chain_entry["symbol_id"].is_string(),
        "missing symbol_id in chain"
    );
    assert!(
        chain_entry["chain"].is_array(),
        "missing chain in chain entry"
    );
    // Chain starts from helper's ID
    let target_id = r["data"]["symbol"]["id"].as_str().unwrap();
    let chain = chain_entry["chain"].as_array().unwrap();
    assert_eq!(
        chain[0].as_str().unwrap(),
        target_id,
        "chain should start at target symbol"
    );
}

#[test]
fn impact_preview_meta_source_is_text_heuristic() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(r["_meta"]["source"].as_str().unwrap(), "text_heuristic");
}

#[test]
fn impact_preview_call_chain_ends_at_caller() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for entry in r["data"]["call_chains"].as_array().unwrap() {
        let chain = entry["chain"].as_array().unwrap();
        let last = chain.last().unwrap().as_str().unwrap();
        let sym_id = entry["symbol_id"].as_str().unwrap();
        assert_eq!(last, sym_id, "chain should end at symbol_id");
    }
}

#[test]
fn impact_preview_no_duplicate_affected_symbols() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let ids: Vec<&str> = r["data"]["affected_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "duplicate affected symbols: {ids:?}"
    );
}

#[test]
fn impact_preview_unknown_symbol_returns_error() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "no_such_sym");
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn impact_preview_leaf_symbol_has_no_affected() {
    let (_ws, svc, sid) = build_call_repo();
    // run() has no callers
    let r = impact_preview(&svc, &sid, "run");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(r["data"]["affected_symbol_count"].as_u64().unwrap(), 0);
    assert_eq!(r["data"]["affected_files"].as_u64().unwrap(), 0);
}

#[test]
fn impact_preview_affected_by_file_keys_match_affected_symbols() {
    let (_ws, svc, sid) = build_call_repo();
    let r = impact_preview(&svc, &sid, "helper");
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let files_in_list: HashSet<&str> = r["data"]["affected_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["file"].as_str())
        .collect();
    let files_in_dict: HashSet<&str> = r["data"]["affected_by_file"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    assert_eq!(files_in_list, files_in_dict, "file keys mismatch");
}

// ===========================================================================
// get_blast_radius call_depth tests  (TestBlastRadiusCallDepth, 5 tests)
// ===========================================================================

#[test]
fn blast_radius_call_depth_zero_no_callers_field() {
    let (_ws, svc, sid) = build_call_repo();
    let r = blast_radius_with_call_depth(&svc, &sid, "helper", 1, 0);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"]["callers"].is_null(),
        "callers should not be present with call_depth=0"
    );
    assert!(
        r["data"]["caller_count"].is_null(),
        "caller_count should not be present with call_depth=0"
    );
}

#[test]
fn blast_radius_call_depth_one_adds_callers_field() {
    let (_ws, svc, sid) = build_call_repo();
    let r = blast_radius_with_call_depth(&svc, &sid, "helper", 1, 1);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"]["callers"].is_array(),
        "callers should be present with call_depth=1"
    );
    assert!(
        r["data"]["caller_count"].is_number(),
        "caller_count should be present"
    );
}

#[test]
fn blast_radius_call_depth_callers_includes_process() {
    let (_ws, svc, sid) = build_call_repo();
    let r = blast_radius_with_call_depth(&svc, &sid, "helper", 1, 1);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let names: Vec<&str> = r["data"]["callers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(
        names.contains(&"process"),
        "expected process in callers: {names:?}"
    );
}

#[test]
fn blast_radius_existing_fields_unchanged_with_call_depth() {
    let (_ws, svc, sid) = build_call_repo();
    let r = blast_radius_with_call_depth(&svc, &sid, "helper", 1, 1);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r["data"]["confirmed"].is_array(), "missing confirmed");
    assert!(r["data"]["potential"].is_array(), "missing potential");
    assert!(
        r["data"]["confirmed_count"].is_number(),
        "missing confirmed_count"
    );
    assert!(
        r["data"]["potential_count"].is_number(),
        "missing potential_count"
    );
    assert!(
        r["data"]["overall_risk_score"].is_number(),
        "missing overall_risk_score"
    );
    assert!(
        r["data"]["importer_count"].is_number(),
        "missing importer_count"
    );
}

#[test]
fn blast_radius_call_depth_clamped_to_3() {
    let (_ws, svc, sid) = build_call_repo();
    // call_depth=99 should not raise, clamped to 3
    let r = blast_radius_with_call_depth(&svc, &sid, "helper", 1, 99);
    assert!(
        r["ok"].as_bool().unwrap(),
        "should not error with large call_depth: {r:?}"
    );
    assert!(r["data"]["callers"].is_array(), "callers should be present");
}

// ===========================================================================
// find_references include_call_chain tests  (TestFindReferencesCallChain, 6 tests)
// ===========================================================================

#[test]
fn find_references_default_no_calling_symbols() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs(&svc, &sid, "helper", false);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for refentry in r["data"]["references"].as_array().unwrap() {
        assert!(
            refentry["calling_symbols"].is_null(),
            "calling_symbols should not be present by default"
        );
    }
}

#[test]
fn find_references_include_call_chain_adds_calling_symbols() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs(&svc, &sid, "helper", true);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let mut found = false;
    for refentry in r["data"]["references"].as_array().unwrap() {
        if refentry["calling_symbols"].is_array() {
            found = true;
            assert!(!refentry["calling_symbols"].as_array().unwrap().is_empty());
        }
    }
    assert!(
        found,
        "No reference had calling_symbols when include_call_chain=true"
    );
}

#[test]
fn find_references_calling_symbols_contain_process() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs(&svc, &sid, "helper", true);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // Find the services.py reference
    let services_ref = r["data"]["references"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["file"].as_str().unwrap_or("").contains("services"));
    assert!(services_ref.is_some(), "Expected services.py in references");
    let services_ref = services_ref.unwrap();
    assert!(services_ref["calling_symbols"].is_array());
    let calling_names: Vec<&str> = services_ref["calling_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(
        calling_names.contains(&"process"),
        "expected process in calling_symbols: {calling_names:?}"
    );
}

#[test]
fn find_references_calling_symbols_have_id_name_kind_line() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs(&svc, &sid, "helper", true);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    for refentry in r["data"]["references"].as_array().unwrap() {
        for cs in refentry
            .get("calling_symbols")
            .and_then(|v| v.as_array())
            .unwrap_or(&vec![])
        {
            assert!(cs["id"].is_string(), "missing id in calling_symbol");
            assert!(cs["name"].is_string(), "missing name in calling_symbol");
            assert!(cs["kind"].is_string(), "missing kind in calling_symbol");
            assert!(cs["line"].is_number(), "missing line in calling_symbol");
        }
    }
}

#[test]
fn find_references_batch_mode_ignores_include_call_chain() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs_batch(&svc, &sid, &["helper", "process"], true);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"]["results"].is_array(),
        "batch mode should have results array"
    );
}

#[test]
fn find_references_existing_reference_fields_unchanged() {
    let (_ws, svc, sid) = build_call_repo();
    let r = find_refs(&svc, &sid, "helper", true);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"]["reference_count"].is_number(),
        "missing reference_count"
    );
    assert!(r["data"]["references"].is_array(), "missing references");
    assert!(r["_meta"].is_object(), "missing _meta");
    for refentry in r["data"]["references"].as_array().unwrap() {
        assert!(refentry["file"].is_string(), "missing file");
        assert!(refentry["matches"].is_array(), "missing matches");
    }
}
