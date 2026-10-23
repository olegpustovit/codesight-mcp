//! Integration test: `read_content` emits `_meta.next_step_hint` only when
//! the caller is going to act on returned bytes (verify or hash). Plain reads
//! stay hint-free to avoid noise.

use std::fs;

use dowser_engine::config::AppConfig;
use dowser_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let idx = tempdir().unwrap();
    fs::create_dir_all(ws.path().join("src")).unwrap();
    fs::write(
        ws.path().join("src/lib.py"),
        "def hinted():\n    return 'hi'\n",
    )
    .unwrap();
    let svc = dowser_mcp::test_support::make_service_from_config(AppConfig::new(
        idx.path().to_path_buf(),
    ))
    .unwrap();
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": ws.path().to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    let sid = ing["data"]["source_id"].as_str().unwrap().to_string();
    (ws, idx, svc, sid)
}

fn read(svc: &CodeService, sid: &str, extra: Value) -> Value {
    let mut base = json!({
        "source_id": sid,
        "refs": [{
            "kind": "file",
            "path": "src/lib.py",
        }],
    });
    if let (Some(base_obj), Some(extra_obj)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in extra_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }
    let r = svc.dispatch("read_content", base).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r
}

#[test]
fn next_step_hint_absent_on_plain_read() {
    let (_ws, _idx, svc, sid) = setup();
    let r = read(&svc, &sid, json!({}));
    assert!(
        r["_meta"].get("next_step_hint").is_none(),
        "plain reads should not emit next_step_hint: {}",
        r["_meta"]
    );
}

#[test]
fn next_step_hint_present_with_verify() {
    let (_ws, _idx, svc, sid) = setup();
    let r = read(&svc, &sid, json!({ "verify": true }));
    let hint = r["_meta"].get("next_step_hint").expect("expected hint");
    assert_eq!(hint["tool"].as_str(), Some("read_content"));
    assert!(
        hint["reason"].as_str().is_some_and(|s| !s.is_empty()),
        "hint must carry a reason: {hint}"
    );
}

#[test]
fn next_step_hint_present_when_hash_requested() {
    let (_ws, _idx, svc, sid) = setup();
    let r = read(&svc, &sid, json!({ "include": ["hash"] }));
    assert!(
        r["_meta"].get("next_step_hint").is_some(),
        "include=[hash] should emit hint: {}",
        r["_meta"]
    );
}
