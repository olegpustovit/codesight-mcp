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
    svc: std::sync::Arc<CodeService>,
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
    build_env_with_preindex(false)
}

fn build_env_with_preindex(preindex: bool) -> ProbeEnv {
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
            ("target.py", "def target():\n    return 1\n"),
            (
                "caller.py",
                "from target import target\ndef caller():\n    return target()\n",
            ),
            (
                "caller2.py",
                "from target import target\nvalue = target()\n",
            ),
            (
                "mentions.py",
                "# target()\ndef mention():\n    return \"target\"\n",
            ),
            ("lines.py", "# NEEDLE\n# NEEDLE\n# NEEDLE\n"),
            (
                "convert.dart",
                "List<num> rgb(List<num> xyz)\n{\n  return [xyz[0] + 1];\n}\n",
            ),
            ("sample.ml", "module M = struct\nlet inc x = x + 1\nend\n"),
            ("sample.mli", "module M : sig\n val inc : int -> int\nend\n"),
            (
                "main.go",
                "package main\nimport \"errors\"\nfunc fail() error { return errors.New(\"x\") }\n",
            ),
            ("httpx/errors.go", "package httpx\nfunc LocalError() {}\n"),
            ("mix.ts", "import './ui';\nimport 'errors';\n"),
            (
                "keys.json",
                r#"{"plain":1,"quote\"key":2,"slash\\key":3,"":4,"\u03b2":5}"#,
            ),
            ("theme.css", ".button { color: red; }\n"),
            (
                "ui.ts",
                "import \"./theme.css\";\nexport function render() {}\n",
            ),
            ("README.md", "# Probe fixture\n\nDescribes regex routing.\n"),
        ],
    );

    // The CUTPLAN adapters resolve source_id from cwd. Use a guard pattern.
    let cwd_guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&repo).unwrap();

    if preindex {
        let r = std::process::Command::new(env!("CARGO_BIN_EXE_codesight-mcp"))
            .env("CODESIGHT_INDEX_DIR", ws.path().join("idx"))
            .args([
                "ingest",
                repo.to_str().unwrap(),
                "--domains",
                "code",
                "--full",
            ])
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        assert_eq!(
            serde_json::from_slice::<Value>(&r.stdout).unwrap()["ok"],
            true
        );
    }
    let svc = std::sync::Arc::new(
        codesight_mcp::test_support::make_service_from_config(AppConfig::new(
            ws.path().join("idx"),
        ))
        .unwrap(),
    );
    svc.install_arc_handle();

    // Pre-warm: ingest under an explicit source_id matching cwd-derived id.
    if !preindex {
        svc.dispatch(
            "ingest_sources",
            json!({
                "path": repo.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    }

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
fn diagnostic_empty_impact_never_establishes_safety() {
    let env = build_env();
    let known = env
        .svc
        .dispatch(
            "impact",
            json!({"analysis":"blast_radius", "ref":"target.py::target#function"}),
        )
        .unwrap();
    assert!(
        known["data"]["symbol_scoped_file_count"].as_u64().unwrap() > 0,
        "{known}"
    );
    for args in [
        json!({"analysis":"blast_radius", "ref":"no_such_identifier_anywhere"}),
        json!({"analysis":"blast_radius", "ref":"target.py::target#function", "limit":0}),
    ] {
        let response = env.svc.dispatch("impact", args).unwrap();
        eprintln!("empty impact: {} ms", response["_meta"]["timing_ms"]);
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(
            response["data"]["symbol_scoped_files"],
            json!([]),
            "{response}"
        );
        assert_eq!(
            response["data"]["symbol_scoped_file_count"], 0,
            "{response}"
        );
        assert_eq!(response["_meta"]["confidence"], "partial", "{response}");
        assert_eq!(response["data"]["status"], "incomplete", "{response}");
        assert_eq!(response["data"]["complete"], false, "{response}");
    }
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
    let cov = data(&r).get("identifier_match_pct").and_then(Value::as_f64);
    assert!(
        cov.is_some(),
        "impact.rename_safety: identifier_match_pct mandatory (P2.3): {r:?}"
    );
    assert!(!complete, "{r}");
    assert!(safe.is_some_and(Value::is_null), "{r}");
    assert_eq!(data(&r)["status"], "undecided");
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

#[test]
fn diagnostic_regex_limits_preserve_matching_lines() {
    let env = build_env();
    for (cap, expected) in [(2, 2), (3, 3)] {
        let r = env
            .svc
            .dispatch(
                "search",
                json!({"mode":"regex", "query":"NEEDLE", "per_file_limit":cap, "limit":20}),
            )
            .unwrap();
        assert_tier("diagnostic.regex", &r, 150.0);
        eprintln!("regex cap {cap}: {} ms", r["_meta"]["timing_ms"]);
        assert_eq!(
            r["data"]["items"].as_array().unwrap().len(),
            expected,
            "{r}"
        );
        assert_eq!(
            r["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["code"] == "TRUNCATED"),
            cap == 2,
            "{r}"
        );
    }
}

#[test]
fn diagnostic_importers_caps_heuristic_guesses_without_resolved_claims() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "references",
            json!({"kind":"importers", "path":"target.py", "limit":1}),
        )
        .unwrap();
    assert_eq!(r["data"]["importers"].as_array().unwrap().len(), 0, "{r}");
    assert_eq!(
        r["data"]["candidate_usage_importers"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{r}"
    );
    assert_tier("diagnostic.importers", &r, 100.0);
    eprintln!("importers: {} ms", r["_meta"]["timing_ms"]);
    assert_eq!(r["data"]["total_importers"], 0, "{r}");
    assert!(
        r["data"]["total_candidate_usage_importers"]
            .as_u64()
            .unwrap()
            >= 2,
        "{r}"
    );
    assert_eq!(r["data"]["evidence"], "resolved_import", "{r}");
    assert!(
        r["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "TRUNCATED"),
        "{r}"
    );
    let r = env
        .svc
        .dispatch(
            "references",
            json!({"kind":"importers", "path":"target.py", "limit":20}),
        )
        .unwrap();
    assert!(
        !r["data"]["importers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "mentions.py"),
        "{r}"
    );
}

#[test]
fn diagnostic_reference_and_impact_exclude_comment_string_only_files() {
    let env = build_env();
    let r = env
        .svc
        .dispatch(
            "references",
            json!({"kind":"symbol", "ref":"target.py::target#function", "include_weak":true}),
        )
        .unwrap();
    assert_tier("diagnostic.references", &r, 100.0);
    eprintln!("references: {} ms", r["_meta"]["timing_ms"]);
    assert!(
        r["data"]["weak_hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["file"] == "mentions.py"),
        "weak mentions must remain accessible: {r}"
    );
    assert!(
        !r["warnings"].to_string().contains("candidate_text_hits"),
        "public warning names an internal-only field: {r}"
    );
    let items = r["data"]["items"].as_array().unwrap();
    assert!(!items.iter().any(|v| v["file"] == "mentions.py"), "{r}");
    assert!(items.iter().any(|v| v["file"] == "caller.py"), "{r}");
    assert!(
        items.iter().any(|v| v["file"] == "caller2.py"),
        "top-level code missing: {r}"
    );
    let r = env
        .svc
        .dispatch(
            "impact",
            json!({"analysis":"blast_radius", "ref":"target.py::target#function"}),
        )
        .unwrap();
    assert_tier("diagnostic.impact", &r, 250.0);
    eprintln!("impact: {} ms", r["_meta"]["timing_ms"]);
    let files = r["data"]["symbol_scoped_files"].as_array().unwrap();
    assert!(!files.iter().any(|v| v == "mentions.py"), "{r}");
    assert!(files.iter().any(|v| v == "caller.py"), "{r}");
    assert!(files.iter().any(|v| v == "caller2.py"), "{r}");
    assert_eq!(r["_meta"]["confidence"], "partial", "{r}");
    assert_eq!(r["data"]["complete"], false, "{r}");
    let weak = env
        .svc
        .dispatch("impact", json!({"analysis":"blast_radius", "ref":"NEEDLE"}))
        .unwrap();
    assert_eq!(weak["data"]["symbol_scoped_files"], json!([]), "{weak}");
    assert!(
        weak["data"]["weak_files_text_mentions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "lines.py"),
        "{weak}"
    );
    assert_eq!(weak["_meta"]["confidence"], "unreliable", "{weak}");
    assert_eq!(weak["data"]["status"], "incomplete", "{weak}");
    assert_eq!(weak["data"]["complete"], false, "{weak}");
}

#[test]
fn diagnostic_rename_safety_never_infers_bindings_from_syntax_ratio() {
    let env = build_env();
    let r = env.svc.dispatch("impact", json!({"analysis":"rename_safety", "ref":"target.py::target#function", "new_name":"unused_name"})).unwrap();
    assert_eq!(r["data"]["complete"], false, "{r}");
    assert!(r["data"]["safe"].is_null(), "{r}");
    assert_eq!(r["data"]["status"], "undecided", "{r}");
    assert_eq!(r["_meta"]["confidence"], "partial", "{r}");
}

#[test]
fn diagnostic_cli_preindexed_service_refreshes_without_safe_stale_reads() {
    let env = build_env_with_preindex(true);
    let args = json!({"refs":["target.py::target#function"]});
    let initial = env.svc.dispatch("read", args.clone()).unwrap();
    assert!(initial.to_string().contains("return 1"), "{initial}");
    fs::write(env._repo.join("target.py"), "def target():\n    return 2\n").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let r = env.svc.dispatch("read", args.clone()).unwrap();
        if r.to_string().contains("return 2") {
            break;
        }
        assert_ne!(
            r["_meta"]["confidence"], "safe",
            "stale response mislabeled safe: {r}"
        );
        assert!(
            r["warnings"].as_array().is_some_and(|w| !w.is_empty()),
            "stale response lacks warning: {r}"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "refresh did not converge: {r}"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn diagnostic_index_scope_and_failed_reads_are_explicit() {
    let env = build_env();
    // A real unsupported extension stays outside the index, even when it exists.
    fs::write(env._repo.join("program.rays"), "module Program\n").unwrap();
    for kind in ["imports", "importers"] {
        let r = env
            .svc
            .dispatch("references", json!({"kind":kind,"path":"program.rays"}))
            .unwrap();
        assert_eq!(r["ok"], false, "{r}");
        assert!(r.to_string().contains("FILE_NOT_INDEXED"), "{r}");
        assert_ne!(r["_meta"]["confidence"], "safe", "{r}");
    }
    let r = env
        .svc
        .dispatch(
            "read",
            json!({"refs":["target.py::target#function", "program.rays::missing#function"]}),
        )
        .unwrap();
    assert_eq!(r["data"]["items"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["data"]["errors"].as_array().unwrap().len(), 1, "{r}");
    assert_ne!(r["_meta"]["confidence"], "safe", "{r}");
    assert!(r["warnings"].to_string().contains("PARTIAL_BATCH"), "{r}");
    let r = env.svc.dispatch("outline", json!({"view":"repo"})).unwrap();
    assert_eq!(r["data"]["coverage"]["scope"], "indexed_files_only", "{r}");
    assert_eq!(
        r["data"]["coverage"]["repository_coverage_complete"], false,
        "{r}"
    );
    assert!(r["data"]["coverage"]["omitted_file_count"].is_null(), "{r}");
    assert!(
        r["warnings"].to_string().contains("INDEX_SCOPE_ONLY"),
        "{r}"
    );
    assert_eq!(r["_meta"]["confidence"], "partial", "{r}");
}

#[test]
fn diagnostic_test_gaps_describes_static_proxy_not_test_absence() {
    let env = build_env();
    let dead = env
        .svc
        .dispatch("health", json!({"analysis":"dead_code"}))
        .unwrap();
    assert_eq!(
        dead["_meta"]["confidence"], "heuristic",
        "coverage annotation upgraded evidence: {dead}"
    );
    assert!(
        dead["warnings"].to_string().contains("INDEX_SCOPE_ONLY"),
        "{dead}"
    );
    let r = env
        .svc
        .dispatch("health", json!({"analysis":"test_gaps"}))
        .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        r["data"]["evidence"]["basis"], "heuristic_test_file_dependency_graph",
        "{r}"
    );
    assert_eq!(
        r["data"]["evidence"]["executed_test_coverage"], false,
        "{r}"
    );
    assert_eq!(
        r["data"]["evidence"]["total_untested_functions_is_proxy"], true,
        "{r}"
    );
    assert!(
        r["data"]["coverage"]["index_file_count"].as_u64().unwrap() > 0,
        "{r}"
    );
    assert_eq!(r["_meta"]["confidence"], "partial", "{r}");
}

#[test]
fn diagnostic_importer_target_needs_no_outgoing_extractor() {
    let env = build_env();
    let r = env
        .svc
        .dispatch("references", json!({"kind":"imports","path":"theme.css"}))
        .unwrap();
    assert_eq!(r["ok"], false, "{r}");
    assert!(
        r.to_string().contains("IMPORT_EXTRACTION_UNSUPPORTED"),
        "{r}"
    );
    let r = env
        .svc
        .dispatch("references", json!({"kind":"importers","path":"theme.css"}))
        .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert!(
        r["data"]["importers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "ui.ts"),
        "{r}"
    );
    assert_eq!(r["_meta"]["confidence"], "partial", "{r}");
}

#[test]
fn diagnostic_import_resolution_guesses_stay_out_of_resolved_and_strict_graphs() {
    let env = build_env();
    let outgoing = env
        .svc
        .dispatch("references", json!({"kind":"imports","path":"main.go"}))
        .unwrap();
    assert_eq!(outgoing["ok"], true, "{outgoing}");
    assert_eq!(outgoing["data"]["imports"], json!([]), "{outgoing}");
    assert_eq!(
        outgoing["data"]["candidate_imports"],
        json!(["httpx/errors.go"]),
        "{outgoing}"
    );
    assert_eq!(outgoing["data"]["complete"], false, "{outgoing}");
    assert_eq!(outgoing["_meta"]["confidence"], "partial", "{outgoing}");
    let capped = env
        .svc
        .dispatch(
            "references",
            json!({"kind":"imports","path":"mix.ts","limit":1}),
        )
        .unwrap();
    assert_eq!(capped["data"]["imports"], json!(["ui.ts"]), "{capped}");
    assert_eq!(capped["data"]["candidate_imports"], json!([]), "{capped}");
    assert_eq!(capped["data"]["total_candidate_imports"], 1, "{capped}");
    assert_eq!(capped["data"]["truncated"], true, "{capped}");
    assert_eq!(capped["warnings"][0]["code"], "TRUNCATED", "{capped}");
    let incoming = env
        .svc
        .dispatch(
            "references",
            json!({"kind":"importers","path":"httpx/errors.go"}),
        )
        .unwrap();
    assert_eq!(incoming["data"]["importers"], json!([]), "{incoming}");
    assert!(
        incoming["data"]["candidate_usage_importers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "main.go"),
        "{incoming}"
    );
    let config = AppConfig::new(env._ws.path().join("idx"));
    let sid = codesight_engine::repo_id::normalize_repo_id(&env._repo);
    let store = codesight_storage::storage::IndexStore::open_existing(config.repo_db_path(&sid))
        .unwrap()
        .unwrap();
    assert!(
        !store
            .dependency_graph_strict()
            .unwrap()
            .get("main.go")
            .is_some_and(|targets| targets.iter().any(|path| path == "httpx/errors.go"))
    );
}

#[test]
fn diagnostic_ingest_phase_logging_is_opt_in_and_omits_source_text() {
    let workspace = tempdir().unwrap();
    let repo = workspace.path().join("repo");
    fs::create_dir(&repo).unwrap();
    fs::write(
        repo.join("probe.rs"),
        "pub fn private_source_marker() -> i32 { 7 }\n",
    )
    .unwrap();
    for enabled in [false, true] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_codesight-mcp"))
            .env(
                "CODESIGHT_INDEX_DIR",
                workspace.path().join(format!("index-{enabled}")),
            )
            .env(
                "CODESIGHT_INGEST_DIAGNOSTICS",
                if enabled { "1" } else { "0" },
            )
            .env("RUST_LOG", "info")
            .args([
                "--log-format",
                "json",
                "ingest",
                repo.to_str().unwrap(),
                "--domains",
                "code",
                "--full",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| {
                event["fields"]["message"] == "ingest diagnostic"
                    || event["fields"]["message"] == "SCM ingest diagnostic"
            })
            .collect();
        if !enabled {
            assert!(events.is_empty());
            continue;
        }
        for phase in [
            "extraction",
            "dependency_graph",
            "reachability",
            "vectors",
            "store",
            "dispatch_roots",
            "extract_file",
            "scm_parse",
            "scm_capture",
            "scm_dedupe",
            "scm_docs",
            "scm_finalize",
            "scm_postprocess",
            "scm_injections",
        ] {
            for suffix in ["start", "end"] {
                assert!(
                    events
                        .iter()
                        .any(|event| event["fields"]["phase"] == format!("{phase}_{suffix}")),
                    "{events:?}"
                );
            }
        }
        assert!(
            events
                .iter()
                .any(|event| event["fields"]["symbols"].as_u64().unwrap_or(0) > 0)
        );
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("private_source_marker")
        );
    }
}

#[test]
fn diagnostic_json_key_outline_ids_round_trip_reads() {
    let env = build_env();
    let outline = env
        .svc
        .dispatch("outline", json!({"view":"file","path":"keys.json"}))
        .unwrap();
    assert_eq!(outline["ok"], true, "{outline}");
    let symbols = outline["data"]["items"].as_array().unwrap();
    assert_eq!(symbols.len(), 5, "{outline}");
    let mut refs = Vec::new();
    for name in ["plain", "quote\"key", "slash\\key", "", "β"] {
        let matches: Vec<_> = symbols
            .iter()
            .filter(|symbol| symbol["name"] == name)
            .collect();
        assert_eq!(matches.len(), 1, "{outline}");
        let id = matches[0]["id"].as_str().unwrap();
        assert_eq!(id, format!("keys.json::{name}#constant"));
        refs.push(id);
    }
    let read = env.svc.dispatch("read", json!({"refs":refs})).unwrap();
    assert_eq!(read["ok"], true, "{read}");
    assert!(read["data"]["errors"].is_null(), "{read}");
    let items = read["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{read}");
    let actual: std::collections::BTreeSet<_> = items
        .iter()
        .map(|item| {
            let pair: serde_json::Map<String, Value> =
                serde_json::from_str(&format!("{{{}}}", item["source"].as_str().unwrap())).unwrap();
            assert_eq!(pair.len(), 1);
            pair.into_iter().next().unwrap().0
        })
        .collect();
    assert_eq!(
        actual,
        ["plain", "quote\"key", "slash\\key", "", "β"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
}

#[test]
fn diagnostic_stamp_change_removes_deleted_paths_and_regrades_old_guesses() {
    use codesight_core::symbols::FactQuality;
    use codesight_engine::indexing::{Indexer, code_index::index_workspace};
    use codesight_storage::storage::IndexStore;
    let workspace = tempdir().unwrap();
    let repo = workspace.path().join("repo");
    fs::create_dir_all(repo.join("httpx")).unwrap();
    fs::write(repo.join("main.go"), "package main\nimport \"errors\"\n").unwrap();
    fs::write(
        repo.join("httpx/errors.go"),
        "package httpx\nfunc LocalError() {}\n",
    )
    .unwrap();
    fs::write(repo.join("gone.go"), "package main\nfunc Gone() {}\n").unwrap();
    let config = AppConfig::new(workspace.path().join("index"));
    config.ensure_layout().unwrap();
    let indexer = std::sync::Arc::new(Indexer::new());
    let first = index_workspace(&config, &repo, &indexer).unwrap();
    let database = config.repo_db_path(&first.summary.repo);
    let store = IndexStore::open_existing(database.clone())
        .unwrap()
        .unwrap();
    {
        let connection = codesight_storage::db::open_rw(&database).unwrap();
        connection
            .execute(
                "UPDATE repo_info SET extractor_version = 'old-resolution-semantics'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE dependency_edges SET import_fact_quality = ?1",
                [FactQuality::BaselineStructural.as_str()],
            )
            .unwrap();
    }
    assert!(
        store.dependency_graph_strict().unwrap()["main.go"]
            .contains(&"httpx/errors.go".to_string())
    );
    fs::remove_file(repo.join("gone.go")).unwrap();
    let refreshed = index_workspace(&config, &repo, &indexer).unwrap();
    assert_eq!(refreshed.delta.files_removed, 1);
    assert!(!store.file_exists("gone.go").unwrap());
    assert!(
        store
            .symbol_record("gone.go::Gone#function")
            .unwrap()
            .is_none()
    );
    assert!(
        !store.dependency_graph_strict().unwrap()["main.go"]
            .contains(&"httpx/errors.go".to_string())
    );
    assert!(
        store.dependency_graph_with_import_quality().unwrap()["main.go"]
            .contains(&("httpx/errors.go".to_string(), FactQuality::Heuristic))
    );
    assert_eq!(
        store.stored_extractor_version().unwrap(),
        indexer.version_stamp()
    );
}

#[test]
fn diagnostic_dart_and_ocaml_public_symbol_reads_include_complete_spans() {
    let env = build_env();
    for (path, name, body) in [
        ("convert.dart", "rgb", "return [xyz[0] + 1];"),
        ("sample.ml", "inc", "x + 1"),
        ("sample.mli", "inc", "int -> int"),
    ] {
        let outline = env
            .svc
            .dispatch("outline", json!({"view":"file","path":path}))
            .unwrap();
        assert_eq!(outline["ok"], true, "{outline}");
        let symbols: Vec<_> = outline["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|symbol| symbol["name"] == name && symbol["kind"] == "function")
            .collect();
        assert_eq!(symbols.len(), 1, "{outline}");
        let id = symbols[0]["id"].as_str().unwrap();
        let r = env
            .svc
            .dispatch("read", json!({"refs":[format!("id:{id}")]}))
            .unwrap();
        assert_eq!(r["ok"], true, "{r}");
        assert!(
            r["data"]["errors"].is_null()
                || r["data"]["errors"].as_array().is_some_and(Vec::is_empty),
            "{r}"
        );
        assert!(r["data"]["items"].to_string().contains(body), "{r}");
    }
}
