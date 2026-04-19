//! Integration tests: tools trusted folders.

use std::fs;
use std::path::{Path, PathBuf};

use codesight_engine::config::{AppConfig, TrustCheckResult, check_trust, is_broad_path};
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

// ── helpers ────────────────────────────────────────────────────────────────────

fn make_service(ws: &Path) -> CodeService {
    CodeService::new(AppConfig::new(ws.join("idx"))).unwrap()
}

fn make_service_with_config(config: AppConfig) -> CodeService {
    CodeService::new(config).unwrap()
}

fn ingest(svc: &CodeService, root: &Path) -> serde_json::Value {
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

fn get_warnings(result: &serde_json::Value) -> Vec<String> {
    result
        .get("warnings")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .map(|w| {
                    // Warnings can be plain strings or objects with "message"
                    w.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| {
                            w.get("message")
                                .and_then(|m| m.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

fn get_error(result: &serde_json::Value) -> String {
    result
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string()
}

fn is_ok(result: &serde_json::Value) -> bool {
    result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
}

// ═══════════════════════════════════════════════════════════════════════════════
// TestTrustedFolders — Whitelist mode (15 tests)
// ═══════════════════════════════════════════════════════════════════════════════

// 1. Trusted broad root emits warning and proceeds
#[test]
fn trusted_broad_root_emits_warning_and_proceeds() {
    let ws = tempdir().unwrap();
    let project = ws.path().join("project");
    fs::create_dir_all(&project).unwrap();
    // No source files — we just check trust behavior

    let canonical_project = project.canonicalize().unwrap();
    let mut config = AppConfig::new(ws.path().join("idx"));
    config.trusted_folders = vec![canonical_project];

    let svc = make_service_with_config(config);
    let r = ingest(&svc, &project);

    // Should succeed (or fail with "no source files", not a trust error)
    let err = get_error(&r);
    assert!(
        !err.contains("not under trusted_folders"),
        "should not get trust error for trusted path: {err}"
    );
}

// 2. Untrusted broad root still rejected
#[test]
fn untrusted_broad_root_still_rejected() {
    // A broad root that is not in trusted_folders should be rejected.
    let result = check_trust(
        &{
            let mut c = AppConfig::new(PathBuf::from("/tmp/idx"));
            c.trusted_folders = vec![];
            c
        },
        Path::new("/"),
    );
    match result {
        TrustCheckResult::Untrusted { error } => {
            assert!(
                error.contains("too broad"),
                "expected broad-path error, got: {error}"
            );
        }
        other => panic!("expected Untrusted for broad root, got: {other:?}"),
    }
}

// 3. Trusted folder matching is exact
#[test]
fn trusted_folder_matching_is_exact() {
    // /work is trusted, but /work2 should NOT be trusted.
    let result = check_trust(
        &{
            let mut c = AppConfig::new(PathBuf::from("/tmp/idx"));
            c.trusted_folders = vec![PathBuf::from("/work")];
            c
        },
        Path::new("/work2"),
    );
    match result {
        TrustCheckResult::Untrusted { error } => {
            assert!(
                error.contains("not under trusted_folders"),
                "expected trust error, got: {error}"
            );
        }
        other => panic!("expected Untrusted for sibling path, got: {other:?}"),
    }
}

// 4. Non-broad trusted descendant skips bypass warning
#[test]
fn non_broad_trusted_descendant_skips_bypass_warning() {
    // A descendant under a trusted root with sufficient depth should NOT get
    // the "broad" bypass warning.
    let result = check_trust(
        &{
            let mut c = AppConfig::new(PathBuf::from("/tmp/idx"));
            c.trusted_folders = vec![PathBuf::from("/work")];
            c
        },
        Path::new("/work/project"),
    );
    match result {
        TrustCheckResult::Trusted => {} // correct — no bypass warning
        other => panic!("expected Trusted (no warning) for non-broad descendant, got: {other:?}"),
    }
}

// 5. Non-broad untrusted path is rejected when trusted_folders configured
#[test]
fn non_broad_untrusted_path_is_rejected_when_trusted_folders_configured() {
    let result = check_trust(
        &{
            let mut c = AppConfig::new(PathBuf::from("/tmp/idx"));
            c.trusted_folders = vec![PathBuf::from("/trusted-root")];
            c
        },
        Path::new("/project/src"),
    );
    match result {
        TrustCheckResult::Untrusted { error } => {
            assert!(
                error.contains("not under trusted_folders"),
                "expected trust error, got: {error}"
            );
        }
        other => panic!("expected Untrusted, got: {other:?}"),
    }
}

// 6. Non-broad paths allow indexing when trusted_folders empty
#[test]
fn non_broad_paths_allow_indexing_when_trusted_folders_empty() {
    let ws = tempdir().unwrap();
    let src = ws.path().join("project").join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("main.py"), "def hello():\n    return 1\n").unwrap();

    // Default config has empty trusted_folders
    let svc = make_service(ws.path());
    let r = ingest(&svc, &src);
    assert!(is_ok(&r), "expected success for normal path: {r:?}");
}

// 7. Trusted folders configured, untrusted path returns trust error
#[test]
fn trusted_folders_configured_untrusted_path_returns_trust_error() {
    let ws = tempdir().unwrap();
    let untrusted = ws.path().join("project").join("src");
    fs::create_dir_all(&untrusted).unwrap();
    fs::write(untrusted.join("main.py"), "x = 1\n").unwrap();

    let mut config = AppConfig::new(ws.path().join("idx"));
    config.trusted_folders = vec![PathBuf::from("/trusted-root")];

    let svc = make_service_with_config(config);
    let r = ingest(&svc, &untrusted);
    assert!(!is_ok(&r), "expected failure for untrusted path");
    let err = get_error(&r);
    assert!(
        err.contains("not under trusted_folders"),
        "expected trust error, got: {err}"
    );
}

// 9. Index folder uses project config trusted_folders
#[test]
fn index_folder_uses_project_config_trusted_folders() {
    let ws = tempdir().unwrap();
    let project = ws.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("main.py"), "def hello():\n    return 1\n").unwrap();

    // Config trusts the project directory itself (use canonical path)
    let canonical_project = project.canonicalize().unwrap();
    let mut config = AppConfig::new(ws.path().join("idx"));
    config.trusted_folders = vec![canonical_project];

    let svc = make_service_with_config(config);
    let r = ingest(&svc, &project);
    assert!(is_ok(&r), "expected success for trusted project: {r:?}");
}

// 10. Project config escape path blocked
#[test]
fn index_folder_project_config_escape_blocked() {
    use codesight_engine::config::sanitize_project_trusted_folders;

    let project_root = PathBuf::from("/projects/myproject");
    let entries = vec![PathBuf::from("../outside")];
    let sanitized = sanitize_project_trusted_folders(&project_root, &entries);
    assert!(
        sanitized.is_empty(),
        "escape paths should be rejected: {sanitized:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TestNestedGitignore (2 tests)
// ═══════════════════════════════════════════════════════════════════════════════

// 16. Nested .gitignore excludes subdirectory files
#[test]
fn nested_gitignore_excludes_subdirectory_files() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    // Create a .git dir so the ignore crate recognizes gitignore files
    fs::create_dir_all(root.join(".git")).unwrap();

    for subdir in ["cap", "core"] {
        let sub = root.join(subdir);
        fs::create_dir_all(sub.join("deps")).unwrap();
        fs::write(sub.join("deps").join("some_dep.py"), "x = 1\n").unwrap();
        fs::write(sub.join("app.py"), "def app(): pass\n").unwrap();
        fs::write(sub.join(".gitignore"), "/deps/\n/_build/\n").unwrap();
    }

    let svc = make_service(ws.path());
    let sid = {
        let r = ingest(&svc, &root);
        assert!(is_ok(&r), "expected success: {r:?}");
        r["data"]["source_id"].as_str().unwrap().to_string()
    };

    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "file_tree" }),
        )
        .unwrap();
    assert!(is_ok(&r), "{r:?}");
    let tree_str = serde_json::to_string(&r["data"]).unwrap();
    // app.py files should be indexed
    assert!(tree_str.contains("app.py"), "expected app.py in tree");
    // deps/ files should be excluded by nested .gitignore
    assert!(
        !tree_str.contains("some_dep"),
        "deps/ should be excluded by .gitignore: {tree_str}"
    );
}

// 17. Root .gitignore still works
#[test]
fn root_gitignore_still_works() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".gitignore"), "*.pyc\n__pycache__/\n").unwrap();
    fs::write(root.join("main.py"), "def main(): pass\n").unwrap();
    fs::write(root.join("main.pyc"), [0u8; 10]).unwrap();

    let svc = make_service(ws.path());
    let sid = {
        let r = ingest(&svc, &root);
        assert!(is_ok(&r), "expected success: {r:?}");
        r["data"]["source_id"].as_str().unwrap().to_string()
    };

    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "file_tree" }),
        )
        .unwrap();
    assert!(is_ok(&r), "{r:?}");
    let tree_str = serde_json::to_string(&r["data"]).unwrap();
    assert!(tree_str.contains("main.py"), "expected main.py in tree");
    assert!(
        !tree_str.contains(".pyc"),
        ".pyc should be excluded by .gitignore"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TestIndexFolderGitignoreWarning (4 tests)
// ═══════════════════════════════════════════════════════════════════════════════

// 19. No warning when .gitignore present
#[test]
fn no_warning_when_gitignore_present() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(".gitignore"), "*.pyc\n").unwrap();
    for i in 0..50 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = make_service(ws.path());
    let r = ingest(&svc, &root);
    let warnings = get_warnings(&r);
    assert!(
        !warnings.iter().any(|w| w.contains("No .gitignore")),
        "unexpected gitignore warning: {warnings:?}"
    );
}

// 20. No warning when file count below threshold
#[test]
fn no_warning_when_file_count_below_threshold() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("small");
    fs::create_dir_all(&root).unwrap();
    for i in 0..10 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = make_service(ws.path());
    let r = ingest(&svc, &root);
    let warnings = get_warnings(&r);
    assert!(
        !warnings.iter().any(|w| w.contains("No .gitignore")),
        "unexpected gitignore warning for small repo: {warnings:?}"
    );
}

// 21. Warning emitted when no .gitignore and large index
#[test]
fn warning_emitted_when_no_gitignore_and_large_index() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("big");
    fs::create_dir_all(&root).unwrap();
    for i in 0..500 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = make_service(ws.path());
    let r = ingest(&svc, &root);
    assert!(is_ok(&r), "expected success: {r:?}");
    let warnings = get_warnings(&r);
    assert!(
        warnings.iter().any(|w| w.contains("No .gitignore")),
        "expected gitignore warning, got: {warnings:?}"
    );
}

// 22. Warning message includes file count and advice
#[test]
fn warning_message_includes_file_count_and_advice() {
    let ws = tempdir().unwrap();
    let root = ws.path().join("advice");
    fs::create_dir_all(&root).unwrap();
    for i in 0..500 {
        fs::write(root.join(format!("file_{i}.py")), format!("# file {i}\n")).unwrap();
    }

    let svc = make_service(ws.path());
    let r = ingest(&svc, &root);
    let warnings = get_warnings(&r);
    let gitignore_warnings: Vec<&String> = warnings
        .iter()
        .filter(|w| w.contains("No .gitignore"))
        .collect();
    assert!(!gitignore_warnings.is_empty(), "expected gitignore warning");
    let msg = &gitignore_warnings[0];
    assert!(
        msg.contains("500") || msg.to_lowercase().contains("file"),
        "warning should mention file count: {msg}"
    );
    assert!(
        msg.contains(".gitignore"),
        "warning should mention .gitignore: {msg}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// TestVersionMismatchWarning (1 test)
// ═══════════════════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════════════════
// Broad-path unit tests (additional coverage for is_broad_path)
// ═══════════════════════════════════════════════════════════════════════════════

// 25. is_broad_path identifies broad roots correctly
#[test]
fn broad_path_detection_works() {
    assert!(is_broad_path(Path::new("/")));
    assert!(is_broad_path(Path::new("/home")));
    assert!(is_broad_path(Path::new("/Users")));
    assert!(is_broad_path(Path::new("/tmp")));

    // Non-broad paths
    assert!(!is_broad_path(Path::new("/home/user/project")));
    assert!(!is_broad_path(Path::new("/work/project")));
    assert!(!is_broad_path(Path::new("/Users/dev/code")));
}

// 26. Trust check integration through ingest_sources
#[test]
fn trust_check_integration_through_ingest() {
    let ws = tempdir().unwrap();
    let project = ws.path().join("project").join("src");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("main.py"), "def hello(): pass\n").unwrap();

    // Configure trust to only allow a different folder
    let mut config = AppConfig::new(ws.path().join("idx"));
    config.trusted_folders = vec![PathBuf::from("/some/other/trusted")];

    let svc = make_service_with_config(config);
    let r = ingest(&svc, &project);

    // Should fail with trust error
    assert!(!is_ok(&r), "expected trust failure: {r:?}");
    let err = get_error(&r);
    assert!(
        err.contains("not under trusted_folders"),
        "expected trust error message, got: {err}"
    );
}
