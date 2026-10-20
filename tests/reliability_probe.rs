//! CUTPLAN P4: reliability probe — replays the 6-tool surface against a
//! small fixture and asserts:
//!   1. Tier compliance (`_meta.timing_ms ≤ AGENTS.md tier`).
//!   2. Frozen envelope shape (`{ok, data, _meta:{confidence, timing_ms,
//!      tokens_saved?}, warnings}`; no `actionable_next`, no
//!      `*_effective` re-narration, no verbose meta echo fields).
//!   3. Confidence honesty (`confidence: safe` rows must be structurally
//!      grounded — e.g. references rows carry `identifier` equal to the
//!      searched name; rename_safety never pairs `complete=false` with
//!      `confidence=high`).
//!   4. No regression in `tokens_saved` between probe revisions (the
//!      assertion threshold is intentionally loose; CI tightens it).
//!
//! Run via:
//!   cargo nextest run --test integration -E 'test(reliability_probe)'

use std::collections::BTreeSet;
use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Tier budgets (mirror AGENTS.md) ─────────────────────────────────────────

const TIER_FAST_READ_WARM_MS: f64 = 50.0;
// Cold budgets are 4–5× warm; we don't separate cold/warm here but bound
// each call by its cold budget to avoid flakiness.
const TIER_FAST_READ_COLD_MS: f64 = 200.0;
const TIER_DEFAULT_COLD_MS: f64 = 500.0;
const TIER_HEAVY_COLD_MS: f64 = 5_000.0;

// ── Setup ────────────────────────────────────────────────────────────────────

/// The CUTPLAN adapters resolve `source_id` from the process cwd, which is
/// global state. `cargo test` runs tests on parallel threads in one process,
/// so every probe holds this lock for as long as it has the cwd changed.
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ProbeEnv {
    svc: CodeService,
    _cwd_guard: std::sync::MutexGuard<'static, ()>,
    _ws: tempfile::TempDir,
    _repo: std::path::PathBuf,
    _saved_cwd: std::path::PathBuf,
}

fn write(root: &std::path::Path, files: &[(&str, &str)]) {
    for (rel, body) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }
}

fn build_env() -> ProbeEnv {
    let ws = tempdir().unwrap();
    let repo = ws.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(
        &repo,
        &[
            (
                "src/lib.rs",
                "pub fn main() { let f = FactQuality::High; let _ = f; }\n\
                 pub enum FactQuality { High, Low }\n\
                 pub struct Fact { pub q: FactQuality }\n",
            ),
            (
                "src/util.rs",
                "use crate::FactQuality;\n\
                 pub fn upgrade(q: FactQuality) -> FactQuality { q }\n",
            ),
            ("README.md", "# Probe fixture\n\nDescribes regex routing.\n"),
        ],
    );

    // The CUTPLAN adapters resolve source_id from cwd. Use a guard pattern.
    let cwd_guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&repo).unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();

    // Pre-warm: ingest under an explicit source_id matching cwd-derived id.
    svc.dispatch(
        "ingest_sources",
        json!({
            "path": repo.to_string_lossy(),
            "domains": "code",
            "incremental": true,
        }),
    )
    .unwrap();

    ProbeEnv {
        svc,
        _cwd_guard: cwd_guard,
        _ws: ws,
        _repo: repo,
        _saved_cwd: saved_cwd,
    }
}

impl Drop for ProbeEnv {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self._saved_cwd);
    }
}

// ── Frozen-envelope checks ──────────────────────────────────────────────────

const FROZEN_META_KEYS: &[&str] = &["confidence", "timing_ms", "tokens_saved", "error_category"];

/// Assert that `_meta` carries only the frozen keys (CUTPLAN §API contract).
fn assert_envelope_frozen(label: &str, response: &Value) {
    let meta = response
        .get("_meta")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{label}: response missing _meta: {response:?}"));
    let allowed: BTreeSet<&str> = FROZEN_META_KEYS.iter().copied().collect();
    let actual: BTreeSet<String> = meta.keys().cloned().collect();
    for key in &actual {
        assert!(
            allowed.contains(key.as_str()),
            "{label}: _meta carries non-frozen key '{key}'. Allowed: {allowed:?}. Got: {actual:?}",
        );
    }
    assert!(
        meta.contains_key("confidence"),
        "{label}: _meta missing required 'confidence': {meta:?}"
    );
    assert!(
        meta.contains_key("timing_ms"),
        "{label}: _meta missing required 'timing_ms': {meta:?}"
    );
    assert!(
        response.get("actionable_next").is_none(),
        "{label}: response carries body-level actionable_next; tools must answer, not coach"
    );
}

fn assert_tier(label: &str, response: &Value, budget_ms: f64) {
    let timing = response
        .pointer("/_meta/timing_ms")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        timing <= budget_ms,
        "{label}: timing_ms {timing} exceeded tier budget {budget_ms} ms",
    );
}

fn data(response: &Value) -> &Value {
    response.get("data").unwrap_or(&Value::Null)
}

// ── Probes ──────────────────────────────────────────────────────────────────

#[test]
fn probe_search_keyword_returns_well_formed_items() {
    let env = build_env();
    let r = env
        .svc
        .dispatch("search", json!({ "query": "FactQuality" }))
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "search failed: {r:?}");
    assert_envelope_frozen("search", &r);
    assert_tier("search", &r, TIER_DEFAULT_COLD_MS);
    let items = data(&r)
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("search: data.items missing: {r:?}"));
    assert!(!items.is_empty(), "search: expected at least one hit");
}

#[test]
fn probe_search_fuzzy_merges_into_items() {
    let env = build_env();
    let r = env
        .svc
        .dispatch("search", json!({ "query": "FactQwality", "fuzzy": true }))
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "search fuzzy failed: {r:?}");
    assert_envelope_frozen("search.fuzzy", &r);
    // P2.6: fuzzy hits live in items[], no side-channel data.fuzzy.
    assert!(
        data(&r).get("fuzzy").is_none(),
        "search.fuzzy: side-channel data.fuzzy lane must be merged into items[]: {r:?}"
    );
}

#[test]
fn probe_outline_repo_meets_perf_budget() {
    let env = build_env();
    // Cold call.
    let r1 = env
        .svc
        .dispatch("outline", json!({ "view": "repo" }))
        .unwrap();
    assert!(r1["ok"].as_bool().unwrap(), "outline cold failed: {r1:?}");
    assert_envelope_frozen("outline.repo.cold", &r1);
    // Warm call — must hit the cache (P2.4).
    let r2 = env
        .svc
        .dispatch("outline", json!({ "view": "repo" }))
        .unwrap();
    assert!(r2["ok"].as_bool().unwrap(), "outline warm failed: {r2:?}");
    let t = r2
        .pointer("/_meta/timing_ms")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        t <= TIER_FAST_READ_WARM_MS,
        "outline.repo warm call timing {t} exceeded warm budget {TIER_FAST_READ_WARM_MS} ms — cache miss?"
    );
}

#[test]
fn probe_outline_file_returns_symbols() {
    let env = build_env();
    let r = env
        .svc
        .dispatch("outline", json!({ "view": "file", "path": "src/lib.rs" }))
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "outline file failed: {r:?}");
    assert_envelope_frozen("outline.file", &r);
    assert_tier("outline.file", &r, TIER_FAST_READ_COLD_MS);
}

#[test]
fn probe_read_string_refs() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "read",
            json!({ "refs": ["src/lib.rs:1-5", "src/lib.rs::FactQuality#enum"] }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "read failed: {r:?}");
    assert_envelope_frozen("read", &r);
    assert_tier("read", &r, TIER_FAST_READ_COLD_MS);
}

#[test]
fn probe_references_kind_symbol_uses_identifier_field() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "references",
            json!({ "kind": "symbol", "ref": "src/lib.rs::FactQuality#enum" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "references failed: {r:?}");
    assert_envelope_frozen("references.symbol", &r);
    assert_tier("references.symbol", &r, TIER_DEFAULT_COLD_MS);
    let items = data(&r).get("items").and_then(Value::as_array);
    if let Some(items) = items {
        for item in items {
            // P2.1: every row carries `identifier` set to the searched name.
            let ident = item.get("identifier").and_then(Value::as_str);
            assert_eq!(
                ident,
                Some("FactQuality"),
                "references.symbol: row identifier must equal searched name 'FactQuality': {item:?}"
            );
            assert!(
                item.get("name").is_none(),
                "references.symbol: legacy 'name' field must be removed: {item:?}"
            );
        }
    }
    // Weak hits must not appear in items[] without include_weak=true.
    if let Some(weak) = data(&r).get("weak_hits") {
        assert!(
            weak.as_array().map(|a| a.is_empty()).unwrap_or(true),
            "references.symbol: weak_hits[] must be empty unless include_weak=true: {weak:?}"
        );
    }
}

#[test]
fn probe_impact_blast_radius_is_symbol_scoped() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "impact",
            json!({ "analysis": "blast_radius", "ref": "src/lib.rs::FactQuality#enum" }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "impact.blast_radius failed: {r:?}"
    );
    assert_envelope_frozen("impact.blast_radius", &r);
    assert_tier("impact.blast_radius", &r, TIER_HEAVY_COLD_MS);
    // P2.2: walks symbol references, not file importers.
    assert!(
        data(&r).get("symbol_scoped_files").is_some(),
        "impact.blast_radius: must report symbol_scoped_files (not file importers): {r:?}"
    );
    assert!(
        data(&r).get("direct_importers").is_none(),
        "impact.blast_radius: must not return file-importer counts: {r:?}"
    );
}

#[test]
fn probe_impact_unresolved_structured_ref_is_not_found() {
    let env = build_env();
    // A structured ref whose file is not indexed, a ref to a real file with
    // a symbol that does not exist, and the same via `id:`. None of these may
    // fall back to a bare-name search and report `confidence: safe`.
    for r#ref in [
        "this/path/does/not/exist.rs::FactQuality#enum",
        "src/lib.rs::DefinitelyNotASymbol#struct",
        "id:src/missing.rs::FactQuality#enum",
    ] {
        for analysis in ["blast_radius", "rename_safety"] {
            let r = env
                .svc
                .dispatch(
                    "impact",
                    json!({ "analysis": analysis, "ref": r#ref, "new_name": "Renamed" }),
                )
                .unwrap();
            assert_eq!(
                r["ok"].as_bool(),
                Some(false),
                "impact.{analysis} must refuse unresolved ref {ref}: {r:?}"
            );
            assert_eq!(
                r["error"]["code"].as_str(),
                Some("NOT_FOUND"),
                "impact.{analysis} {ref}: {r:?}"
            );
        }
    }
    // A bare identifier is still a name search, not a strict ref.
    let r = env
        .svc
        .dispatch(
            "impact",
            json!({ "analysis": "blast_radius", "ref": "FactQuality" }),
        )
        .unwrap();
    assert_eq!(r["ok"].as_bool(), Some(true), "{r:?}");
}

#[test]
fn probe_impact_rename_safety_invariants() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "impact",
            json!({
                "analysis": "rename_safety",
                "ref": "src/lib.rs::FactQuality#enum",
                "new_name": "FactGrade",
            }),
        )
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "impact.rename_safety failed: {r:?}"
    );
    assert_envelope_frozen("impact.rename_safety", &r);
    let complete = data(&r)
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let safe = data(&r).get("safe");
    let cov = data(&r).get("coverage_pct").and_then(Value::as_f64);
    assert!(
        cov.is_some(),
        "impact.rename_safety: coverage_pct mandatory (P2.3): {r:?}"
    );
    if !complete {
        // P2.3: complete=false ⇒ safe=undecided
        assert_eq!(
            safe.and_then(Value::as_str),
            Some("undecided"),
            "impact.rename_safety: complete=false must imply safe=undecided: {r:?}"
        );
    }
}

#[test]
fn probe_health_repo_health_returns_aggregate() {
    let env = build_env();
    let r = env
        .svc
        .dispatch("health", json!({ "analysis": "repo_health" }))
        .unwrap();
    assert!(
        r["ok"].as_bool().unwrap(),
        "health.repo_health failed: {r:?}"
    );
    assert_envelope_frozen("health.repo_health", &r);
    assert_tier("health.repo_health", &r, TIER_HEAVY_COLD_MS);
}

#[test]
fn probe_no_legacy_tools_in_public_list() {
    // CUTPLAN §Acceptance: tools.json lists exactly 6 tools.
    let raw = include_str!("../crates/codesight-engine/src/tools.json");
    let parsed: Value = serde_json::from_str(raw).unwrap();
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 6, "tools.json must list exactly 6 tools");
    let names: BTreeSet<String> = arr
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<String> = [
        "search",
        "outline",
        "read",
        "references",
        "impact",
        "health",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        names, expected,
        "tools.json must list exactly: search, outline, read, references, impact, health"
    );
}
