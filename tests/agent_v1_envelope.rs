//! Integration tests: agent-v1 envelope and error paths.

use codesight_engine::config::AppConfig;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn unknown_tool_returns_envelope_error() {
    let dir = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(
        dir.path().to_path_buf(),
    ))
    .unwrap();
    let r = svc.dispatch("nonexistent_tool", json!({})).unwrap();
    assert_eq!(r["ok"], false);
    assert!(r.get("error").is_some());
    assert!(r.get("_meta").is_some());
}
