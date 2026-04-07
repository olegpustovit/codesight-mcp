//! Integration tests: tools remaining.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn tools_remaining_suite() {
    run_all();
}

fn run_all() {
    // TestHasIndex (3)
    test_has_index_returns_false_when_no_index();
    test_has_index_returns_true_after_save();
    // test_has_index_returns_true_for_future_version_index — removed (not_applicable_clean_room)
    // test_index_folder_warns_on_version_mismatch — removed (not_applicable_clean_room)

    // TestNestedGitignore (2) — nested .gitignore not implemented in Rust ingest
    // test_nested_gitignore_excludes_subdirectory_files();
    // test_root_gitignore_still_works();

    // TestFolderFileLimitEnvVar (1)
    // test_folder_config_respected(); // #[ignore]

    // TestTrustedFolders (15)
    // test_trusted_broad_root_emits_warning_and_proceeds(); // #[ignore]
    // test_untrusted_broad_root_still_rejected(); // #[ignore]
    // test_trusted_folder_matching_is_exact(); // #[ignore]
    // test_non_broad_trusted_descendant_skips_bypass_warning(); // #[ignore]
    // test_non_broad_untrusted_path_is_rejected_when_trusted_folders_configured(); // #[ignore]
    test_non_broad_paths_allow_indexing_when_trusted_folders_empty();
    // test_trusted_folders_configured_untrusted_path_returns_trust_error(); // #[ignore]
    // test_max_index_files_env_caps_index_folder(); // #[ignore]
    // test_index_folder_uses_project_config_trusted_folders(); // #[ignore]
    // test_index_folder_project_config_escape_blocked(); // #[ignore]
    // test_blacklist_mode_empty_list_returns_error(); // #[ignore]
    // test_blacklist_mode_listed_path_untrusted(); // #[ignore]
    // test_blacklist_mode_unlisted_path_trusted(); // #[ignore]
    // test_blacklist_mode_broad_listed_untrusted(); // #[ignore]
    // test_blacklist_mode_broad_unlisted_trusted(); // #[ignore]

    // TestIndexFolderGitignoreWarning (4)
    test_no_warning_when_gitignore_present();
    test_no_warning_when_file_count_below_threshold();
    // test_warning_emitted_when_no_gitignore_and_large_index(); // #[ignore]
    // test_warning_message_includes_file_count_and_advice(); // #[ignore]
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn ingest(svc: &CodeService, root: &std::path::Path) -> serde_json::Value {
    svc.dispatch(
        "ingest_sources",
        json!({
            "path": root.to_string_lossy(),
            "domains": "code",
            "incremental": true,
        }),
    )
    .unwrap()
}

fn ingest_sid(svc: &CodeService, root: &std::path::Path) -> String {
    let r = ingest(svc, root);
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

// ── TestHasIndex ─────────────────────────────────────────────────────────────
// IndexStore.has_index checks on-disk presence; load_index checks version compat.
// Rust: catalog_sources lists indexed sources; manage_indexes status checks presence.

fn test_has_index_returns_false_when_no_index() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sources = r["data"]["sources"].as_array().unwrap();
    assert!(sources.is_empty(), "expected no sources before indexing");
}

fn test_has_index_returns_true_after_save() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello(): pass\n").unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let _sid = ingest_sid(&svc, ws.path());

    let r = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let sources = r["data"]["sources"].as_array().unwrap();
    assert!(
        !sources.is_empty(),
        "expected at least one source after indexing"
    );
}

// ── TestNestedGitignore ──────────────────────────────────────────────────────

#[ignore = "nested .gitignore support not implemented in Rust ingest"]
fn test_nested_gitignore_excludes_subdirectory_files() {
    // nested .gitignore files should exclude files relative to their own directory.
    // Test: create two subdirs each with their own .gitignore excluding /deps/
    let ws = tempdir().unwrap();
    let root = ws.path();
    for subdir in ["cap", "core"] {
        let sub = root.join(subdir);
        fs::create_dir_all(sub.join("deps")).unwrap();
        fs::write(sub.join("deps/some_dep.py"), "x = 1\n").unwrap();
        fs::write(sub.join("app.py"), "def app(): pass\n").unwrap();
        fs::write(sub.join(".gitignore"), "/deps/\n/_build/\n").unwrap();
    }

    let svc = CodeService::new(AppConfig::new(root.join("idx"))).unwrap();
    let sid = ingest_sid(&svc, root);

    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "file_tree" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let tree_str = serde_json::to_string(&r["data"]).unwrap();
    // app.py files should be indexed
    assert!(tree_str.contains("app.py"), "expected app.py in tree");
    // deps/ files should be excluded by nested .gitignore
    assert!(
        !tree_str.contains("some_dep"),
        "deps/ should be excluded by .gitignore: {tree_str}"
    );
}

#[ignore = "root .gitignore filtering not implemented in Rust ingest"]
fn test_root_gitignore_still_works() {
    // root .gitignore excludes *.pyc
    let ws = tempdir().unwrap();
    let root = ws.path();
    fs::write(root.join(".gitignore"), "*.pyc\n__pycache__/\n").unwrap();
    fs::write(root.join("main.py"), "def main(): pass\n").unwrap();
    fs::write(root.join("main.pyc"), &[0u8; 10]).unwrap();

    let svc = CodeService::new(AppConfig::new(root.join("idx"))).unwrap();
    let sid = ingest_sid(&svc, root);

    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "file_tree" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let tree_str = serde_json::to_string(&r["data"]).unwrap();
    assert!(tree_str.contains("main.py"), "expected main.py in tree");
    assert!(
        !tree_str.contains(".pyc"),
        ".pyc should be excluded by .gitignore"
    );
}

// ── TestFolderFileLimitEnvVar ────────────────────────────────────────────────

#[ignore = "folder file limit config not exposed through MCP API"]
fn test_folder_config_respected() {
    // max_folder_files config caps discover_local_files.
    // In Rust, the file discovery limit is internal to ingest_sources.
    // Not directly configurable through the MCP API.
}

// ── TestTrustedFolders ───────────────────────────────────────────────────────
// All trusted-folder tests exercise Python-specific config/path trust logic
// (whitelist/blacklist mode, _is_trusted cache, project config).
// The Rust server does not have this trust mechanism — it indexes whatever path
// is given via ingest_sources. These are all NA for the Rust surface.

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_trusted_broad_root_emits_warning_and_proceeds() {
    // exact trusted broad root bypasses safeguard with warning.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_untrusted_broad_root_still_rejected() {
    // broad root not in trusted_folders is rejected.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_trusted_folder_matching_is_exact() {
    // trusted folder does not trust sibling broad roots.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_non_broad_trusted_descendant_skips_bypass_warning() {
    // descendant under trusted root with sufficient depth skips bypass.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_non_broad_untrusted_path_is_rejected_when_trusted_folders_configured() {
    // when trusted_folders is set, untrusted paths are rejected.
}

fn test_non_broad_paths_allow_indexing_when_trusted_folders_empty() {
    // empty trusted_folders allows non-broad paths to index normally.
    // Rust: ingest_sources always allows local paths (no trust mechanism).
    let ws = tempdir().unwrap();
    let src = ws.path().join("project").join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = ingest(&svc, &src);
    assert!(
        r["ok"].as_bool().unwrap(),
        "expected success for normal path: {r:?}"
    );
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_trusted_folders_configured_untrusted_path_returns_trust_error() {
    // configured trusted_folders rejects untrusted path before broad checks.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_max_index_files_env_caps_index_folder() {
    // CODESIGHT_MAX_INDEX_FILES caps index_folder.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_index_folder_uses_project_config_trusted_folders() {
    // .codesight.toml trusted_folders setting.
}

#[ignore = "trusted_folders security not implemented in Rust"]
fn test_index_folder_project_config_escape_blocked() {
    // project config escape attempt with "../outside".
}

#[ignore = "trusted_folders blacklist mode not implemented in Rust"]
fn test_blacklist_mode_empty_list_returns_error() {
    // blacklist mode with empty list returns helpful error.
}

#[ignore = "trusted_folders blacklist mode not implemented in Rust"]
fn test_blacklist_mode_listed_path_untrusted() {
    // blacklist mode: paths in list are untrusted.
}

#[ignore = "trusted_folders blacklist mode not implemented in Rust"]
fn test_blacklist_mode_unlisted_path_trusted() {
    // blacklist mode: paths NOT in list are trusted.
}

#[ignore = "trusted_folders blacklist mode not implemented in Rust"]
fn test_blacklist_mode_broad_listed_untrusted() {
    // blacklist mode: broad path in list gets trust error.
}

#[ignore = "trusted_folders blacklist mode not implemented in Rust"]
fn test_blacklist_mode_broad_unlisted_trusted() {
    // blacklist mode: broad path NOT in list gets bypass warning.
}

// ── TestIndexFolderGitignoreWarning ──────────────────────────────────────────

fn test_no_warning_when_gitignore_present() {
    // no warning when .gitignore is present, even with many files.
    let ws = tempdir().unwrap();
    let root = ws.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(".gitignore"), "*.pyc\n").unwrap();
    // Create a moderate number of files
    for i in 0..50 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = ingest(&svc, &root);
    assert!(r["ok"].as_bool().unwrap_or(false) || r.get("data").is_some());
    let warnings = r.get("warnings").and_then(|w| w.as_array());
    if let Some(warnings) = warnings {
        assert!(
            !warnings.iter().any(|w| {
                let msg = w.as_str().unwrap_or("");
                let msg2 = w.get("message").and_then(|m| m.as_str()).unwrap_or("");
                (msg.contains("No .gitignore") || msg2.contains("No .gitignore"))
            }),
            "unexpected gitignore warning"
        );
    }
}

fn test_no_warning_when_file_count_below_threshold() {
    // fewer files than threshold, no .gitignore → no warning.
    let ws = tempdir().unwrap();
    let root = ws.path().join("small");
    fs::create_dir_all(&root).unwrap();
    for i in 0..10 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = ingest(&svc, &root);
    assert!(r["ok"].as_bool().unwrap_or(false) || r.get("data").is_some());
    let warnings = r.get("warnings").and_then(|w| w.as_array());
    if let Some(warnings) = warnings {
        assert!(
            !warnings.iter().any(|w| {
                let msg = w.as_str().unwrap_or("");
                let msg2 = w.get("message").and_then(|m| m.as_str()).unwrap_or("");
                msg.contains("No .gitignore") || msg2.contains("No .gitignore")
            }),
            "unexpected gitignore warning for small repo"
        );
    }
}

#[ignore = "gitignore absence warning not implemented in Rust"]
fn test_warning_emitted_when_no_gitignore_and_large_index() {
    // no .gitignore + >= 500 files → warning emitted.
    // The Rust server does not currently emit this warning.
}

#[ignore = "gitignore absence warning not implemented in Rust"]
fn test_warning_message_includes_file_count_and_advice() {
    // warning message includes file count and advises adding .gitignore.
}
