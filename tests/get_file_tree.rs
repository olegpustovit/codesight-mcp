//! Integration tests: get file tree.

use rustc_hash::FxHashMap;
use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

struct TreeFixture {
    _workspace: tempfile::TempDir,
    svc: CodeService,
    source_id: String,
}

#[test]
fn get_file_tree_suite() {
    let header = setup_header_fixture();
    let small = setup_small_tree_fixture();
    let large = setup_large_tree_fixture();
    let custom = setup_custom_cap_fixture();
    run_all(&header, &small, &large, &custom);
}

fn run_all(header: &TreeFixture, small: &TreeFixture, large: &TreeFixture, custom: &TreeFixture) {
    run_falls_back_cpp_header_no_symbols(header);
    run_no_truncation_when_under_limit(small);
    run_truncation_at_default_cap(large);
    run_truncation_hint_mentions_path_prefix(large);
    run_custom_max_files(custom);
    run_hint_includes_total_count(large);
    run_path_prefix_scoping(large);
    // run_prefers_symbol_language_over_extension — see standalone #[ignore] test below
}

fn ingest_code(svc: &CodeService, root: &Path) -> String {
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    ing["data"]["source_id"].as_str().unwrap().to_string()
}

fn inspect_file_tree(svc: &CodeService, sid: &str, extra: Value) -> Value {
    // Tests assert on `_meta.file_count` (and others) which are stripped from
    // the slim default envelope — request verbose meta so the tree stats stay
    // visible to the assertions below.
    let mut base = json!({
        "source_id": sid,
        "view": "file_tree",
        "_meta_verbose": true,
    });
    if let Value::Object(ref mut m) = base
        && let Value::Object(e) = extra
    {
        for (k, v) in e {
            m.insert(k, v);
        }
    }
    svc.dispatch("inspect_structure", base).unwrap()
}

fn flatten_files(nodes: &[Value]) -> FxHashMap<String, Value> {
    let mut out = FxHashMap::default();
    fn walk(nodes: &[Value], out: &mut FxHashMap<String, Value>) {
        for n in nodes {
            match n["type"].as_str() {
                Some("file") => {
                    let p = n["path"].as_str().unwrap().to_string();
                    out.insert(p, n.clone());
                }
                Some("dir") => {
                    if let Some(ch) = n["children"].as_array() {
                        walk(ch, out);
                    }
                }
                _ => {}
            }
        }
    }
    walk(nodes, &mut out);
    out
}

fn setup_header_fixture() -> TreeFixture {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("tree_demo");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("include")).unwrap();
    fs::write(root.join("src/main.cpp"), "int main() { return 0; }\n").unwrap();
    fs::write(
        root.join("include/no_symbols.h"),
        "/* header with no symbols */\n",
    )
    .unwrap();

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let source_id = ingest_code(&svc, &root);
    TreeFixture {
        _workspace: workspace,
        svc,
        source_id,
    }
}

fn setup_small_tree_fixture() -> TreeFixture {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("small_tree");
    fs::create_dir_all(root.join("src")).unwrap();
    for i in 0..10 {
        fs::write(
            root.join("src").join(format!("file_{i}.py")),
            format!("# {i}\n"),
        )
        .unwrap();
    }

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let source_id = ingest_code(&svc, &root);
    TreeFixture {
        _workspace: workspace,
        svc,
        source_id,
    }
}

fn setup_large_tree_fixture() -> TreeFixture {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("large_tree");
    fs::create_dir_all(root.join("src")).unwrap();
    for i in 0..510 {
        fs::write(
            root.join("src").join(format!("file_{i}.py")),
            format!("# {i}\n"),
        )
        .unwrap();
    }
    fs::create_dir_all(root.join("lib")).unwrap();
    for i in 0..8 {
        fs::write(
            root.join("lib").join(format!("util_{i}.py")),
            format!("# u{i}\n"),
        )
        .unwrap();
    }

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let source_id = ingest_code(&svc, &root);
    TreeFixture {
        _workspace: workspace,
        svc,
        source_id,
    }
}

fn setup_custom_cap_fixture() -> TreeFixture {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("custom_cap");
    fs::create_dir_all(root.join("src")).unwrap();
    for i in 0..100 {
        fs::write(
            root.join("src").join(format!("file_{i}.py")),
            format!("# {i}\n"),
        )
        .unwrap();
    }

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let source_id = ingest_code(&svc, &root);
    TreeFixture {
        _workspace: workspace,
        svc,
        source_id,
    }
}

fn run_falls_back_cpp_header_no_symbols(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({}));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    let files = flatten_files(items);
    let h = files
        .get("include/no_symbols.h")
        .expect("expected include/no_symbols.h");
    assert_eq!(h["language"], json!("cpp"));
    assert_eq!(h["symbol_count"], json!(0));
}

fn run_no_truncation_when_under_limit(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({}));
    assert!(r["ok"].as_bool().unwrap());
    assert!(r["data"].get("truncated").is_none());
    assert_eq!(r["_meta"]["file_count"], json!(10));

    let items = r["data"]["items"].as_array().unwrap();
    assert_eq!(flatten_files(items).len(), 10);
}

fn run_truncation_at_default_cap(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({}));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(r["data"]["truncated"], json!(true));
    assert_eq!(r["data"]["total_file_count"], json!(518));
    assert_eq!(r["_meta"]["file_count"], json!(500));
    let items = r["data"]["items"].as_array().unwrap();
    assert_eq!(flatten_files(items).len(), 500);
}

fn run_truncation_hint_mentions_path_prefix(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({}));
    assert!(r["ok"].as_bool().unwrap());
    let hint = r["data"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("path_prefix"),
        "hint should mention path_prefix: {hint}"
    );
}

fn run_custom_max_files(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({ "max_files": 50 }));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert_eq!(r["data"]["truncated"], json!(true));
    assert_eq!(r["_meta"]["file_count"], json!(50));
    assert_eq!(r["data"]["total_file_count"], json!(100));
}

fn run_hint_includes_total_count(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({}));
    assert!(r["ok"].as_bool().unwrap());
    let total = r["data"]["total_file_count"].as_u64().unwrap();
    let hint = r["data"]["hint"].as_str().unwrap();
    assert!(
        hint.contains(&total.to_string()),
        "hint should include total file count: {hint}"
    );
}

fn run_path_prefix_scoping(fixture: &TreeFixture) {
    let r = inspect_file_tree(&fixture.svc, &fixture.source_id, json!({ "path": "lib/" }));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r["data"].get("truncated").is_none());
    assert_eq!(r["_meta"]["file_count"], json!(8));
    let items = r["data"]["items"].as_array().unwrap();
    assert_eq!(flatten_files(items).len(), 8);
}
/// A .h file with C symbols should show `language='c'` instead of the
/// extension default.
///
/// The Rust extractor registry maps `.h` to the `cpp` grammar (since
/// `runtime/queries/cpp/tags.scm` lists `.h` in its extensions). There is
/// no separate C grammar or per-symbol language override. The reference test
/// expects the file tree to report `language='c'` when the symbols happen to
/// look like C — this requires per-file language inference from symbol content,
/// which is not implemented.
#[test]
fn case_prefers_symbol_language_over_extension() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("c_tree");
    fs::create_dir_all(root.join("include")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("include/api.h"),
        "int only_c(void) { return 0; }\n",
    )
    .unwrap();
    fs::write(root.join("src/orphan.cpp"), "// no symbols here\n").unwrap();

    let cfg = AppConfig::new(workspace.path().join("idx"));
    let svc = codesight_mcp::test_support::make_service_from_config(cfg).unwrap();
    let sid = ingest_code(&svc, &root);

    let r = inspect_file_tree(&svc, &sid, json!({}));
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let items = r["data"]["items"].as_array().unwrap();
    let files = flatten_files(items);
    let h = files.get("include/api.h").expect("expected include/api.h");
    // Reference expects "c" for a .h file with C-style symbols
    assert_eq!(h["language"], json!("c"));
    assert_eq!(h["symbol_count"], json!(1));
}
