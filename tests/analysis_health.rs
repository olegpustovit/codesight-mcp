//! FEEDBACK.md §7: `manage_indexes action=analysis_health` surfaces
//! reachability + graph counters so the "dead-code false positive" class
//! is observable via the tool itself rather than by eyeballing output.

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use codesight_mcp::test_support::{index_code_fixture, write_files};
use serde_json::json;
use tempfile::tempdir;

fn make_service(index_root: &std::path::Path) -> CodeService {
    CodeService::new(AppConfig::new(index_root.to_path_buf())).unwrap()
}

#[test]
fn analysis_health_reports_graph_and_reachability_shape() {
    let ws = tempdir().unwrap();
    // Synthetic workspace with a binary crate importing a lib crate — same
    // fixture shape as tests/self_dogfood.rs so workspace_crate_edges is
    // guaranteed non-zero after Phase 1.
    write_files(
        ws.path(),
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers=[\"crates/bin\",\"crates/lib\"]\nresolver=\"2\"\n",
            ),
            (
                "crates/bin/Cargo.toml",
                "[package]\nname=\"bin\"\nversion=\"0.1.0\"\nedition=\"2024\"\n\n[dependencies]\nlib = { path = \"../lib\" }\n",
            ),
            (
                "crates/bin/src/main.rs",
                "use lib::api;\nuse missing_crate::unresolved;\nuse another_missing::absent;\nfn main() { api(); }\n",
            ),
            (
                "crates/lib/Cargo.toml",
                "[package]\nname=\"lib\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
            ),
            ("crates/lib/src/lib.rs", "pub fn api() {}\n"),
            ("tools/check.py", "import json\n"),
            ("fixtures/json.rs", "pub fn placeholder() {}\n"),
        ],
    )
    .unwrap();

    let (_config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let svc = make_service(&ws.path().join("idx"));

    let resp = svc
        .dispatch(
            "manage_indexes",
            json!({"action": "analysis_health", "source_id": sid}),
        )
        .unwrap();
    assert!(resp["ok"].as_bool().unwrap_or(false), "{resp}");
    let data = &resp["data"];

    assert_eq!(data["action"], json!("analysis_health"));
    assert_eq!(data["source_id"], json!(sid));

    // Shape invariants.
    let reach = &data["reachability"];
    assert!(reach["roots"].as_u64().unwrap() >= 1, "{reach}");
    assert!(reach["indexed_files"].as_u64().unwrap() >= 2, "{reach}");

    let graph = &data["graph"];
    assert!(graph["total_edges"].as_u64().is_some());
    assert!(graph["workspace_crate_edges"].as_u64().is_some());

    assert_eq!(graph["heuristic_import_edges"], json!(1), "{graph}");
    assert_eq!(graph["heuristic_edges_in_strict_graph"], json!(0));
    assert_eq!(graph["unresolved_import_coverage"], json!("not_measured"));
    // One Python candidate coexists with two absent Rust specifiers: stored
    // candidate count cannot measure unresolved imports in either language.

    let stale = &data["staleness"];
    assert!(stale["in_flight"].as_bool().is_some());
}

#[test]
fn analysis_health_errors_without_source_id() {
    let ws = tempdir().unwrap();
    let svc = make_service(&ws.path().join("idx"));
    let resp = svc
        .dispatch("manage_indexes", json!({"action": "analysis_health"}))
        .unwrap();
    assert_eq!(resp["ok"], json!(false), "{resp}");
}
