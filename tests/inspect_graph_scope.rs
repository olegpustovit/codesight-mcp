//! FEEDBACK.md §6b: `inspect_graph` {imports,importers} accept scope /
//! exclude / file_pattern filters so a broad query over a heavily-used
//! symbol doesn't dump the whole workspace.

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_mcp::test_support::{index_code_fixture, write_files};
use serde_json::json;
use tempfile::tempdir;

fn make_service(index_root: &std::path::Path) -> CodeService {
    CodeService::new(AppConfig::new(index_root.to_path_buf())).unwrap()
}

/// Build a mini workspace where three Rust files all import the same target.
fn build_importer_fixture(ws: &std::path::Path) -> String {
    write_files(
        ws,
        &[
            ("Cargo.toml", "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n"),
            ("src/lib.rs", "pub mod target;\npub mod a;\npub mod b;\npub mod c;\n"),
            ("src/target.rs", "pub fn api() {}\n"),
            ("src/a.rs", "use crate::target;\npub fn use_a() { target::api(); }\n"),
            ("src/b.rs", "use crate::target;\npub fn use_b() { target::api(); }\n"),
            ("src/c.rs", "use crate::target;\npub fn use_c() { target::api(); }\n"),
        ],
    )
    .unwrap();
    let (_cfg, sid, _) = index_code_fixture(&ws.join("idx"), ws).unwrap();
    sid
}

#[test]
fn importers_scope_narrows_to_matching_files() {
    let ws = tempdir().unwrap();
    let sid = build_importer_fixture(ws.path());
    let svc = make_service(&ws.path().join("idx"));

    // Unfiltered — lib.rs + {a,b,c}.rs all carry edges to target.rs.
    let unfiltered = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "src/target.rs",
            }),
        )
        .unwrap();
    assert_eq!(unfiltered["data"]["importer_count"].as_u64().unwrap(), 4);

    // With `scope: "src/a"` — only `src/a.rs` qualifies.
    let scoped = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "src/target.rs",
                "scope": "src/a",
            }),
        )
        .unwrap();
    assert_eq!(scoped["data"]["importer_count"].as_u64().unwrap(), 1);
    assert_eq!(
        scoped["data"]["importers"][0].as_str().unwrap(),
        "src/a.rs",
    );
}

#[test]
fn importers_file_pattern_accepts_glob() {
    let ws = tempdir().unwrap();
    let sid = build_importer_fixture(ws.path());
    let svc = make_service(&ws.path().join("idx"));

    let resp = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "src/target.rs",
                "file_pattern": "**/b.rs",
            }),
        )
        .unwrap();
    assert_eq!(resp["data"]["importer_count"].as_u64().unwrap(), 1);
    assert_eq!(
        resp["data"]["importers"][0].as_str().unwrap(),
        "src/b.rs",
    );
}

#[test]
fn importers_exclude_drops_matching_files() {
    let ws = tempdir().unwrap();
    let sid = build_importer_fixture(ws.path());
    let svc = make_service(&ws.path().join("idx"));

    let resp = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "src/target.rs",
                "exclude": ["src/a.rs", "src/c.rs", "src/lib.rs"],
            }),
        )
        .unwrap();
    assert_eq!(resp["data"]["importer_count"].as_u64().unwrap(), 1);
    assert_eq!(
        resp["data"]["importers"][0].as_str().unwrap(),
        "src/b.rs",
    );
}
