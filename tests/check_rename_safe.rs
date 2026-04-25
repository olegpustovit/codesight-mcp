//! Slice tests for rename safety, plus explicit MCP error/meta checks.

use codesight_analysis::impact::rename_safety_payload;
use codesight_engine::config::AppConfig;
use codesight_mcp::test_support::{
    find_symbol_id, index_code_fixture, open_code_store, write_files,
};
use codesight_storage::storage::IndexStore;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn check_rename_safe_suite() {}

fn build_repo() -> (tempfile::TempDir, AppConfig, String, IndexStore) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            (
                "utils.py",
                "def helper():\n    return 42\n\ndef other():\n    return 0\n",
            ),
            ("main.py", "from utils import helper\n\nresult = helper()\n"),
            (
                "cli.py",
                "from main import result\nfrom utils import other\nprint(result)\n",
            ),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    (ws, config, sid, store)
}

#[test]
fn test_safe_rename_returns_true() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "compute", false)
        .unwrap()
        .unwrap();
    assert!(r["safe"].as_bool().unwrap());
    assert!(r["conflicts"].as_array().unwrap().is_empty());
    assert!(r["checked_files"].as_u64().unwrap() >= 1);
}

#[test]
fn test_collision_detected() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "other", false)
        .unwrap()
        .unwrap();
    assert!(!r["safe"].as_bool().unwrap());
    let conflicts = r["conflicts"].as_array().unwrap();
    assert!(!conflicts.is_empty());
    let names: Vec<&str> = conflicts
        .iter()
        .filter_map(|c| c["existing_name"].as_str())
        .collect();
    assert!(names.contains(&"other"), "{names:?}");
}

#[test]
fn test_missing_symbol_returns_none_from_domain() {
    let (_ws, _config, _sid, store) = build_repo();
    assert!(
        rename_safety_payload(&store, "nonexistent_symbol_xyz", "foo", false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn test_missing_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": "no_such_repo",
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": "x",
                    "new_name": "y",
                }],
            }),
        )
        .unwrap();
    assert!(
        !r["ok"].as_bool().unwrap_or(true),
        "expected error for missing repo: {r:?}"
    );
}

#[test]
fn trait_method_rename_is_flagged_unsafe_with_impl_sites() {
    // P0.3: renaming a Rust trait method must enumerate every `impl Trait for
    // X { fn method(...) }` site and report `safe: false` unless the caller
    // opted into the impl rewrite.
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            (
                "trait_def.rs",
                r#"
pub trait Greeter {
    fn greet(&self) -> String;
}
"#,
            ),
            (
                "impl_english.rs",
                r#"
use crate::trait_def::Greeter;
pub struct English;
impl Greeter for English {
    fn greet(&self) -> String {
        "hi".into()
    }
}
"#,
            ),
            (
                "impl_french.rs",
                r#"
use crate::trait_def::Greeter;
pub struct French;
impl Greeter for French {
    fn greet(&self) -> String {
        "salut".into()
    }
}
"#,
            ),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    let sym = find_symbol_id(&store, "greet").expect("found greet");
    let r = rename_safety_payload(&store, &sym, "say_hi", false)
        .unwrap()
        .unwrap();
    assert_eq!(
        r["safe"].as_bool(),
        Some(false),
        "trait method rename must not be `safe: true`: {r}",
    );
    let td = &r["trait_dispatch"];
    assert_eq!(
        td["detected_trait_owner"].as_str(),
        Some("Greeter"),
        "expected Greeter as owning trait: {td}",
    );
    let impl_count = td["impl_site_count"].as_u64().unwrap_or(0);
    // The defining file reported on `symbol.file` is the starting rename site
    // and is not duplicated in `required_edits`. Everything else — the other
    // impl block plus the trait declaration — must appear.
    assert!(
        impl_count >= 2,
        "expected >=2 impl sites (the other impl + the trait decl), got {impl_count}: {td}",
    );
    let defining_file = r["symbol"]["file"].as_str().unwrap();
    let required: Vec<&str> = td["required_edits"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["file"].as_str())
        .collect();
    assert!(
        required.iter().any(|f| f.contains("trait_def.rs")),
        "trait_def.rs should be in required_edits: {required:?}",
    );
    let other_impl = if defining_file.contains("english") {
        "impl_french.rs"
    } else {
        "impl_english.rs"
    };
    assert!(
        required.iter().any(|f| f.contains(other_impl)),
        "{other_impl} should be in required_edits (defining={defining_file}): {required:?}",
    );
}

#[test]
fn test_result_includes_symbol_info() {
    let (_ws, _config, _sid, store) = build_repo();
    let sym = find_symbol_id(&store, "helper").unwrap();
    let r = rename_safety_payload(&store, &sym, "foo", false)
        .unwrap()
        .unwrap();
    assert_eq!(r["symbol"]["name"].as_str().unwrap(), "helper");
    assert_eq!(r["new_name"].as_str().unwrap(), "foo");
}

#[test]
fn test_timing_present() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            ("utils.py", "def helper():\n    return 42\n"),
            ("main.py", "from utils import helper\nresult = helper()\n"),
        ],
    )
    .unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": src.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = indexed["data"]["source_id"].as_str().unwrap().to_string();
    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "helper",
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 5,
            }),
        )
        .unwrap();
    let sym = search["data"]["items"][0]["ref"]["id"].as_str().unwrap();
    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": indexed["data"]["source_id"],
                "analysis": "rename_safety",
                "refs": [{
                    "kind": "symbol",
                    "stable_id": sym,
                    "new_name": "foo",
                }],
            }),
        )
        .unwrap();
    assert!(r.get("_meta").is_some(), "_meta missing: {r:?}");
    assert!(r["_meta"]["timing_ms"].is_number(), "timing_ms missing");
}
