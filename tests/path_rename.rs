use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn ingest_rust_repo(files: &[(&str, &str)]) -> (CodeService, tempfile::TempDir, String) {
    let ws = tempdir().unwrap();
    for (rel, content) in files {
        let path = ws.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        ws.path().join("idx"),
    ))
    .unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (svc, ws, sid)
}

#[test]
fn path_rename_dry_run_and_apply_rewrites_paths() {
    let (svc, _ws, sid) = ingest_rust_repo(&[
        (
            "src/storage/mod.rs",
            "#[derive(Clone)]\npub struct DocStore;\n",
        ),
        (
            "src/lib.rs",
            "pub mod storage;\npub fn consume(store: crate::storage::DocStore) -> crate::storage::DocStore { store }\n",
        ),
        (
            "src/a.rs",
            "use crate::storage::DocStore;\npub fn make() -> DocStore { DocStore }\n",
        ),
        (
            "src/partial.rs",
            "use crate::{storage::DocStore, util::Thing};\n",
        ),
    ]);

    let preview = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "path_rename",
                "from": "crate::storage",
                "to": "codesight_storage::storage",
                "scope": "src",
            }),
        )
        .unwrap();
    assert!(
        preview["ok"].as_bool().unwrap(),
        "preview failed: {preview:?}"
    );
    assert!(preview["data"]["summary"]["files"].as_u64().unwrap() >= 2);
    assert!(preview["data"]["summary"]["changes"].as_u64().unwrap() >= 2);
    assert!(
        !preview["data"]["summary"]["unhandled"]
            .as_array()
            .unwrap()
            .is_empty(),
        "partial grouped import should be reported"
    );

    let applied = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "path_rename",
                "from": "crate::storage",
                "to": "codesight_storage::storage",
                "scope": "src",
                "apply": true,
            }),
        )
        .unwrap();
    assert!(
        applied["ok"].as_bool().unwrap(),
        "apply failed: {applied:?}"
    );
    assert!(applied["data"]["applied"].as_u64().unwrap() >= 2);
    assert_eq!(
        applied["data"]["complexity_budget"]["verdict"].as_str(),
        Some("unchanged")
    );

    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "codesight_storage::storage::DocStore",
                "query_mode": "regex",
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(search["ok"].as_bool().unwrap(), "search failed: {search:?}");
    assert!(
        !search["data"]["items"].as_array().unwrap().is_empty(),
        "applied rename should be searchable: {search:?}"
    );
}

#[test]
fn path_rename_apply_refuses_duplicate_import_collisions() {
    let (svc, _ws, sid) = ingest_rust_repo(&[
        ("src/storage/mod.rs", "pub struct DocStore;\n"),
        (
            "src/lib.rs",
            "use codesight_storage::storage::DocStore;\nuse crate::storage::DocStore;\n",
        ),
    ]);

    let applied = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "path_rename",
                "from": "crate::storage",
                "to": "codesight_storage::storage",
                "apply": true,
            }),
        )
        .unwrap();
    assert!(!applied["ok"].as_bool().unwrap_or(true));
    assert_eq!(applied["error"]["code"], "PATH_RENAME_COLLISION");
}
