use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::db;
use codesight_mcp::service::CodeService;
use serde_json::json;

#[test]
fn indexed_symbols_persist_extraction_tiers() {
    let workspace = tempfile::tempdir().unwrap();
    let repo_root = workspace.path().join("repo");
    fs::create_dir_all(&repo_root).unwrap();
    fs::write(
        repo_root.join("api.py"),
        "def hello(name):\n    return name\n",
    )
    .unwrap();
    fs::write(
        repo_root.join("schema.sql"),
        "CREATE TABLE users (\n    id INTEGER PRIMARY KEY\n);\n",
    )
    .unwrap();

    let cfg = AppConfig::new(workspace.path().join("index"));
    let svc = CodeService::new(cfg.clone()).unwrap();
    let indexed = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": repo_root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(indexed["ok"].as_bool().unwrap(), "{indexed:?}");

    let repo = indexed["data"]["source_id"].as_str().unwrap();
    let conn = db::open_ro(&cfg.repo_db_path(repo)).unwrap();
    let mut stmt = conn
        .prepare("SELECT language, extraction_tier FROM symbols ORDER BY language, name")
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();

    let tiers: Vec<(String, String)> = rows.map(|row| row.unwrap()).collect();
    assert!(
        tiers.contains(&("python".to_string(), "curated_query_pack".to_string())),
        "missing python curated tier: {tiers:?}"
    );
    assert!(
        tiers.contains(&("sql".to_string(), "curated_query_pack".to_string())),
        "missing sql curated tier: {tiers:?}"
    );
}
