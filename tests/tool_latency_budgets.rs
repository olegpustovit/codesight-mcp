//! Regression guard: every major handler must complete within its
//! documented budget against a synthetic repo. Catches new N+1 / cold-cache
//! / quadratic-loop regressions before they hit production.
//!
//! Budgets are set conservatively above current p95 headroom. They are
//! intentionally lower than the 2000 ms handler cap so we notice when we
//! halve the margin, not just when we tip past it.
//!
//! Reproducers for past regressions (keep these cases present):
//!   - `evaluate_quality analysis=refactor_targets top_files=30` — used to
//!     N+1 on `store.reference_count_and_files` (full-table `instr()` scan
//!     per candidate). Fixed by moving to an in-memory sweep.
//!   - `evaluate_impact analysis=impact_preview` + `inspect_graph graph=calls`
//!     — used to build the O(N²) call-edge matrix on every request. Fixed
//!     by identifier-tokenizing each caller source once against a
//!     `name → callees` index; per-caller BFS replaced with a single BFS
//!     that yields a parent map.

use std::fs;
use std::time::Instant;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

/// Build a mid-size synthetic repo that exercises the same code paths
/// as a real codebase: symbols that reference each other, multi-file
/// import graph, nested functions / classes, and enough mass to make
/// an accidental O(N²) show up as a visible regression.
fn build_fixture(parent: &std::path::Path) -> std::path::PathBuf {
    let src = parent.join("src");
    fs::create_dir_all(&src).unwrap();

    // Generate a file with N small functions that cross-call one
    // another. 40 files × 10 fns = 400 symbols; each fn body references
    // a handful of names from the same + adjacent files so both the
    // call-edge and reference-count paths see realistic work.
    let files_per_module = 40;
    let fns_per_file = 10;
    for f in 0..files_per_module {
        let mut src_text = String::new();
        src_text.push_str(&format!("// file {f}\n"));
        for i in 0..fns_per_file {
            let name = format!("op_{f}_{i}");
            let next = format!("op_{f}_{}", (i + 1) % fns_per_file);
            let cross = format!("op_{}_{i}", (f + 1) % files_per_module);
            src_text.push_str(&format!(
                "pub fn {name}(x: u64) -> u64 {{\n    \
                 let y = {next}(x);\n    \
                 let z = {cross}(y);\n    \
                 y.wrapping_add(z)\n\
                 }}\n\n"
            ));
        }
        fs::write(src.join(format!("mod_{f}.rs")), src_text).unwrap();
    }
    src
}

fn setup() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let root = build_fixture(ws.path());
    let idx_root = ws.path().join("idx");
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(idx_root)).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap_or(false),
        "fixture ingest failed: {r:?}"
    );
    let source_id = r["data"]["source_id"].as_str().unwrap().to_string();
    (ws, svc, source_id)
}

/// Dispatch a tool call and assert it completes within `budget_ms`.
/// The budget includes serialization and index staleness checks but
/// excludes test fixture setup.
fn within_budget(
    svc: &CodeService,
    tool: &str,
    args: Value,
    budget_ms: u128,
    case_label: &str,
) -> Value {
    let t0 = Instant::now();
    let r = svc
        .dispatch(tool, args)
        .unwrap_or_else(|e| panic!("{case_label}: dispatch error: {e}"));
    let elapsed = t0.elapsed().as_millis();
    let ok = r["ok"].as_bool().unwrap_or(false);
    assert!(
        ok,
        "{case_label}: tool returned ok=false: {}",
        r.get("error").cloned().unwrap_or(json!(null))
    );
    assert!(
        elapsed <= budget_ms,
        "{case_label}: {elapsed}ms exceeded budget of {budget_ms}ms (tool={tool})"
    );
    r
}

#[test]
fn tool_latency_budgets_suite() {
    let (_ws, svc, sid) = setup();

    // ── hotspot fast path: trivially indexable reads ──────────────────────
    within_budget(&svc, "catalog_sources", json!({}), 500, "catalog_sources");
    within_budget(
        &svc,
        "inspect_structure",
        json!({"source_id": sid, "view": "repo_outline", "max_tokens": 2000}),
        500,
        "is_repo_outline",
    );
    within_budget(
        &svc,
        "inspect_structure",
        json!({"source_id": sid, "view": "file_tree"}),
        500,
        "is_file_tree",
    );

    // ── search paths ──────────────────────────────────────────────────────
    within_budget(
        &svc,
        "search_catalog",
        json!({
            "source_id": sid, "query": "op_",
            "query_mode": "keyword", "unit_kinds": ["symbol"], "limit": 20,
        }),
        500,
        "search_keyword",
    );

    // ── graph paths (regression canaries: these used to TIMEOUT) ─────────
    // Pick a high-ish fan-in target (op_0_0 is called from op_0_1 via same-
    // file walk and from op_39_0 via cross-file wrap; whole-word matching
    // by tokens sees ~40-80 callers in the synthetic repo).
    let hot_symbol = "src/op_0_0#function".to_string();
    // Resolve the actual stable_id via search, since the mod.rs prefix varies.
    let resolve = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid, "query": "op_0_0",
                "query_mode": "keyword", "unit_kinds": ["symbol"], "limit": 1,
            }),
        )
        .unwrap();
    let hot_stable_id = resolve["data"]["items"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|it| it.get("ref"))
        .and_then(|r| r.get("id"))
        .and_then(|s| s.as_str())
        .map(String::from)
        .unwrap_or(hot_symbol);

    within_budget(
        &svc,
        "inspect_graph",
        json!({"source_id": sid, "graph": "calls", "symbol_id": hot_stable_id}),
        1500,
        "ig_calls_regression_canary_O_N_squared_call_edges",
    );

    within_budget(
        &svc,
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "impact_preview",
            "refs": [{"kind": "symbol", "stable_id": hot_stable_id}],
        }),
        1500,
        "ei_impact_preview_regression_canary_per_caller_BFS",
    );

    within_budget(
        &svc,
        "evaluate_impact",
        json!({
            "source_id": sid,
            "analysis": "blast_radius",
            "refs": [{"kind": "symbol", "stable_id": hot_stable_id}],
        }),
        1000,
        "ei_blast_radius",
    );

    // ── quality analyses (another N+1 regression canary) ─────────────────
    within_budget(
        &svc,
        "evaluate_quality",
        json!({"source_id": sid, "analysis": "quality_signal"}),
        1500,
        "eq_quality_signal",
    );
    within_budget(
        &svc,
        "evaluate_quality",
        json!({"source_id": sid, "analysis": "hotspots", "top_n": 20}),
        800,
        "eq_hotspots",
    );
    within_budget(
        &svc,
        "evaluate_quality",
        json!({"source_id": sid, "analysis": "test_gaps", "top_n": 25, "max_tokens": 3000}),
        800,
        "eq_test_gaps",
    );
    // Post-PERF2.1: handler runs ≤ 200 ms p95 on this fixture (148 ms p95 on
    // the 4.5k-symbol real repo). Budget is 500 ms so a regression that
    // reintroduces the full-table `instr()` scan OR the quadratic
    // cand × other substring sweep fails loudly well before the old 1500 ms
    // threshold. The shared `all_symbols_cached` path + identifier
    // tokenizer are both load-bearing.
    within_budget(
        &svc,
        "evaluate_quality",
        json!({
            "source_id": sid,
            "analysis": "refactor_targets",
            "top_files": 30,
        }),
        500,
        "eq_refactor_targets_regression_canary_per_symbol_instr_scan",
    );
    within_budget(
        &svc,
        "evaluate_quality",
        json!({"source_id": sid, "analysis": "repo_health"}),
        1500,
        "eq_repo_health",
    );

    // ── read paths ───────────────────────────────────────────────────────
    within_budget(
        &svc,
        "read_content",
        json!({
            "source_id": sid,
            "refs": [{"kind": "symbol", "stable_id": hot_stable_id}],
        }),
        500,
        "rc_single_symbol",
    );

    // ── N+1 batching canaries ────────────────────────────────────────────
    // These cases exercise the exact hot loops that used to do `N` single-
    // key DB lookups (`file_outline`/`file_language`/`file_bytes`/
    // `symbol_record`/`symbol_names_by_id`/`symbol_signature_by_id`).
    // Each is now one batch query; a regression that reintroduces the
    // per-key calls would blow past these budgets on the 400-symbol
    // fixture.
    let all_file_paths: Vec<String> = (0..40).map(|f| format!("src/mod_{f}.rs")).collect();

    // `inspect_structure view=file_outline file_paths=[...]` batches
    // `file_outline` + `file_language` — 2 queries total regardless of N.
    within_budget(
        &svc,
        "inspect_structure",
        json!({
            "source_id": sid,
            "view": "file_outline",
            "file_paths": all_file_paths,
        }),
        500,
        "is_file_outline_batch_40_paths_canary",
    );

    // `assemble_context mode=context_bundle refs=[...]` batches 4
    // different per-ref lookups into 3 queries regardless of N.
    let mut many_refs: Vec<Value> = Vec::new();
    for f in 0..10 {
        let q = svc
            .dispatch(
                "search_catalog",
                json!({
                    "source_id": sid, "query": format!("op_{f}_0"),
                    "query_mode": "keyword", "unit_kinds": ["symbol"], "limit": 1,
                }),
            )
            .unwrap();
        if let Some(id) = q["data"]["items"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|it| it.get("ref"))
            .and_then(|r| r.get("id"))
            .and_then(|s| s.as_str())
        {
            many_refs.push(json!({"kind": "symbol", "stable_id": id}));
        }
    }
    within_budget(
        &svc,
        "assemble_context",
        json!({
            "source_id": sid,
            "mode": "context_bundle",
            "refs": many_refs,
            "max_tokens": 8000,
        }),
        1000,
        "ac_context_bundle_10_refs_canary_4N_to_3",
    );

    // ── Cohort C: cross-source scaling canary ────────────────────────────
    // `catalog_sources` iterates every registered source and reads
    // `repo_summary()` + `root_path()` through the registry. Prior to the
    // `SourceRegistry` cache, this reopened a SQLite handle per source on
    // every call; with the registry warm, package_names is an `Arc::clone`
    // off a `RwLock::read`.
    within_budget(
        &svc,
        "catalog_sources",
        json!({}),
        500,
        "cs_catalog_sources_cold",
    );
    within_budget(
        &svc,
        "catalog_sources",
        json!({}),
        300,
        "cs_catalog_sources_warm_cohort_c_canary_source_registry",
    );
}
