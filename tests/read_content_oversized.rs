//! FEEDBACK.md §3a regression: oversized `read_content` responses must
//! per-ref-stub the later refs instead of dumping a JSON-escaped preview.

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_mcp::test_support::{index_code_fixture, write_files};
use serde_json::{Value, json};
use tempfile::tempdir;

fn make_service_with(index_root: &std::path::Path) -> CodeService {
    CodeService::new(AppConfig::new(index_root.to_path_buf())).unwrap()
}

fn big_rust_file(lines: usize) -> String {
    // Each line takes ~30 bytes. 500 lines ≈ 15 KB, matching the FEEDBACK
    // fixture scale. The symbol extractor captures `pub fn` — anything
    // inside is irrelevant for the payload shape.
    let mut s = String::with_capacity(lines * 32);
    s.push_str("pub fn demo() -> u32 {\n");
    for i in 0..lines {
        s.push_str(&format!("    let value_{i} = {i}u32;\n"));
    }
    s.push_str("    0\n}\n");
    s
}

#[test]
fn oversized_read_stubs_later_refs_with_structured_suggestion() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    let big = big_rust_file(500);
    write_files(
        &src,
        &[
            (
                "Cargo.toml",
                "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n",
            ),
            ("src/a.rs", &big),
            ("src/b.rs", &big),
            ("src/c.rs", &big),
        ],
    )
    .unwrap();

    let (_config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let svc = make_service_with(&ws.path().join("idx"));

    // 2000 tokens ≈ 8 KB byte budget: well below the 45 KB of three files,
    // so the second and third files must be stubbed.
    let args = json!({
        "source_id": sid,
        "refs": [
            {"kind": "file", "path": "src/a.rs"},
            {"kind": "file", "path": "src/b.rs"},
            {"kind": "file", "path": "src/c.rs"},
        ],
        "max_tokens": 2000,
    });
    let resp = svc.dispatch("read_content", args).unwrap();
    assert!(resp["ok"].as_bool().unwrap_or(false), "{resp}");
    let items: &Vec<Value> = resp["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "expected one item per ref");

    // First item was read in full (has content).
    assert!(
        items[0].get("content").is_some()
            || items[0]
                .get("truncated")
                .and_then(|b| b.as_bool())
                .is_none(),
        "first ref should be full: {}",
        items[0]
    );

    // Second and third items are stubs with structured suggestions.
    for item in &items[1..] {
        assert_eq!(item["truncated"], json!(true), "expected stub: {item}");
        assert_eq!(
            item["reason"],
            json!("token_budget_exceeded"),
            "expected token_budget_exceeded reason: {item}",
        );
        assert_eq!(
            item["suggestion"]["tool"],
            json!("inspect_structure"),
            "expected inspect_structure suggestion: {item}",
        );
        assert_eq!(
            item["suggestion"]["args"]["view"],
            json!("file_outline"),
            "expected file_outline view: {item}",
        );
    }

    // _meta must carry `truncated` + `truncated_by = ["token_budget"]`.
    let meta = &resp["_meta"];
    assert_eq!(meta["truncated"], json!(true), "_meta.truncated: {meta}");
    assert_eq!(
        meta["truncated_by"],
        json!(["token_budget"]),
        "_meta.truncated_by: {meta}",
    );

    // Warning list must carry one TOKEN_HEAVY_READ per stubbed ref.
    let warnings = resp["warnings"].as_array().unwrap();
    let token_heavy_count = warnings
        .iter()
        .filter(|w| w["code"] == json!("TOKEN_HEAVY_READ"))
        .count();
    assert!(
        token_heavy_count >= 2,
        "expected a TOKEN_HEAVY_READ per trimmed ref, got {} in {:?}",
        token_heavy_count,
        warnings,
    );
}

#[test]
fn under_budget_reads_all_refs() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    write_files(
        &src,
        &[
            (
                "Cargo.toml",
                "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n",
            ),
            ("src/a.rs", "pub fn a() {}\n"),
            ("src/b.rs", "pub fn b() {}\n"),
        ],
    )
    .unwrap();
    let (_config, sid, _) = index_code_fixture(&ws.path().join("idx"), &src).unwrap();
    let svc = make_service_with(&ws.path().join("idx"));

    let resp = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [
                    {"kind": "file", "path": "src/a.rs"},
                    {"kind": "file", "path": "src/b.rs"},
                ],
                "max_tokens": 4000,
            }),
        )
        .unwrap();
    assert!(resp["ok"].as_bool().unwrap_or(false), "{resp}");
    let items = resp["data"]["items"].as_array().unwrap();
    for item in items {
        assert!(
            item.get("truncated").is_none(),
            "should not stub under budget: {item}",
        );
    }
    assert!(
        resp["_meta"].get("truncated").is_none(),
        "_meta.truncated must be unset when every ref fit: {}",
        resp["_meta"],
    );
}
