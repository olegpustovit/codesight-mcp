//! Integration tests: cross repo.

use std::fs;
use tempfile::tempdir;

use codesight_mcp::config::AppConfig;
use codesight_mcp::indexing::packages::{
    RepoEntry, build_package_registry, extract_package_names, extract_root_package_from_specifier,
    find_repos_for_package,
};
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::IndexStore;
use serde_json::json;

// ── helpers ──────────────────────────────────────────────────────────────

fn _write(dir: &std::path::Path, name: &str, content: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

// ============================================================
// Phase 1: extract_package_names — per ecosystem
// ============================================================

// --- Python ---

#[test]
fn case_extract_package_names_python_pyproject_toml_name() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "pyproject.toml",
        "[project]\nname = \"my-lib\"\n",
    );
    _write(tmp.path(), "src/my_lib/__init__.py", "");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.contains(&"my-lib".to_string()), "names = {names:?}");
}

#[test]
fn case_extract_package_names_python_underscore_normalized() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "pyproject.toml",
        "[project]\nname = \"my_lib\"\n",
    );
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"my-lib".to_string()),
        "PEP 503: underscores -> hyphens, got {names:?}"
    );
}

#[test]
fn case_extract_package_names_python_setup_cfg_fallback() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "setup.cfg", "[metadata]\nname = requests\n");
    _write(tmp.path(), "src/requests/__init__.py", "");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.contains(&"requests".to_string()), "names = {names:?}");
}

#[test]
fn case_extract_package_names_python_pyproject_wins_over_setup_cfg() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "pyproject.toml",
        "[project]\nname = \"alpha\"\n",
    );
    _write(tmp.path(), "setup.cfg", "[metadata]\nname = beta\n");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"alpha".to_string()),
        "pyproject.toml should win, got {names:?}"
    );
}

// --- JavaScript ---

#[test]
fn case_extract_package_names_js_package_json_name() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "package.json",
        r#"{"name": "my-package", "version": "1.0.0"}"#,
    );
    _write(tmp.path(), "index.js", "");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"my-package".to_string()),
        "names = {names:?}"
    );
}

#[test]
fn case_extract_package_names_js_scoped_package_json() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "package.json", r#"{"name": "@org/toolkit"}"#);
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"@org/toolkit".to_string()),
        "names = {names:?}"
    );
}

// --- Go ---

#[test]
fn case_extract_package_names_go_go_mod_module() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "go.mod",
        "module github.com/myorg/myrepo\n\ngo 1.21\n",
    );
    _write(tmp.path(), "main.go", "package main\n");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"github.com/myorg/myrepo".to_string()),
        "names = {names:?}"
    );
}

// --- Rust ---

#[test]
fn case_extract_package_names_rust_cargo_toml_name() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
    );
    _write(tmp.path(), "src/lib.rs", "");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.contains(&"my-crate".to_string()), "names = {names:?}");
}

// --- C# ---

#[test]
fn case_extract_package_names_csharp_csproj_package_name() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "MyLib.csproj",
        "<Project><PropertyGroup><PackageName>MyLib.Core</PackageName></PropertyGroup></Project>",
    );
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(
        names.contains(&"MyLib.Core".to_string()),
        "names = {names:?}"
    );
}

#[test]
fn case_extract_package_names_csharp_csproj_assembly_name_fallback() {
    let tmp = tempdir().unwrap();
    _write(
        tmp.path(),
        "MyLib.csproj",
        "<Project><PropertyGroup><AssemblyName>MyLib</AssemblyName></PropertyGroup></Project>",
    );
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.contains(&"MyLib".to_string()), "names = {names:?}");
}

// --- Edge cases ---

#[test]
fn case_extract_package_names_no_manifest_returns_empty() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "main.py", "print('hello')\n");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.is_empty(), "expected empty, got {names:?}");
}

#[test]
fn case_extract_package_names_malformed_pyproject_toml_does_not_raise() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "pyproject.toml", "this is not valid toml ][[[");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    // Should not panic; returns list (possibly empty)
    assert!(names.is_empty() || !names.is_empty()); // Just checking no panic
}

#[test]
fn case_extract_package_names_malformed_package_json_does_not_raise() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "package.json", "{broken json");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.is_empty() || !names.is_empty()); // No panic
}

#[test]
fn case_extract_package_names_empty_manifest_returns_empty() {
    let tmp = tempdir().unwrap();
    _write(tmp.path(), "pyproject.toml", "");
    let names = extract_package_names(tmp.path().to_str().unwrap());
    assert!(names.is_empty(), "expected empty, got {names:?}");
}

// ============================================================
// Phase 2: extract_root_package_from_specifier
// ============================================================

// --- Python ---

#[test]
fn case_extract_root_package_python_simple_module() {
    assert_eq!(
        extract_root_package_from_specifier("flask", "python"),
        "flask"
    );
}

#[test]
fn case_extract_root_package_python_dotted_module() {
    assert_eq!(
        extract_root_package_from_specifier("flask.blueprints", "python"),
        "flask"
    );
}

#[test]
fn case_extract_root_package_python_relative_import_returns_empty() {
    assert_eq!(extract_root_package_from_specifier(".utils", "python"), "");
}

#[test]
fn case_extract_root_package_python_relative_import_multi_dot_returns_empty() {
    assert_eq!(
        extract_root_package_from_specifier("..models", "python"),
        ""
    );
}

#[test]
fn case_extract_root_package_python_pure_relative_returns_empty() {
    assert_eq!(extract_root_package_from_specifier("...", "python"), "");
}

// --- JavaScript ---

#[test]
fn case_extract_root_package_js_unscoped_package() {
    assert_eq!(
        extract_root_package_from_specifier("react", "javascript"),
        "react"
    );
}

#[test]
fn case_extract_root_package_js_unscoped_with_subpath() {
    assert_eq!(
        extract_root_package_from_specifier("lodash/merge", "javascript"),
        "lodash"
    );
}

#[test]
fn case_extract_root_package_js_scoped_package() {
    assert_eq!(
        extract_root_package_from_specifier("@org/package", "javascript"),
        "@org/package"
    );
}

#[test]
fn case_extract_root_package_js_scoped_with_subpath() {
    assert_eq!(
        extract_root_package_from_specifier("@org/package/utils", "javascript"),
        "@org/package"
    );
}

#[test]
fn case_extract_root_package_js_relative_import_returns_empty() {
    assert_eq!(
        extract_root_package_from_specifier("./local", "typescript"),
        ""
    );
}

#[test]
fn case_extract_root_package_js_relative_dotdot_returns_empty() {
    assert_eq!(
        extract_root_package_from_specifier("../sibling", "typescript"),
        ""
    );
}

// --- Go ---

#[test]
fn case_extract_root_package_go_three_segment_module() {
    assert_eq!(
        extract_root_package_from_specifier("github.com/gin-gonic/gin/router", "go"),
        "github.com/gin-gonic/gin"
    );
}

#[test]
fn case_extract_root_package_go_short_module() {
    assert_eq!(extract_root_package_from_specifier("fmt", "go"), "fmt");
}

// --- Rust ---

#[test]
fn case_extract_root_package_rust_crate_path() {
    assert_eq!(
        extract_root_package_from_specifier("serde::de::Deserialize", "rust"),
        "serde"
    );
}

#[test]
fn case_extract_root_package_rust_simple_crate() {
    assert_eq!(
        extract_root_package_from_specifier("tokio", "rust"),
        "tokio"
    );
}

// ============================================================
// Phase 3: package_names field round-trip
// ============================================================

#[test]
fn case_package_names_saved_and_loaded() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&src, "mylib.py", "def hello(): pass\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _sid = ingest(&svc, &src);

    // Re-open the store and check package_names
    let repos_dir = tmp.path().join("idx").join("repos");
    if repos_dir.exists() {
        for entry in fs::read_dir(&repos_dir).unwrap() {
            let entry = entry.unwrap();
            let db_path = entry.path().join("index.sqlite3");
            if db_path.exists() {
                let store = IndexStore::open(db_path).unwrap();
                let names: Vec<String> = store.get_package_names().unwrap();
                assert!(
                    names.contains(&"mylib".to_string()),
                    "expected 'mylib' in package_names, got {names:?}"
                );
                return;
            }
        }
    }
    panic!("No index store found after indexing");
}

#[test]
fn case_old_index_without_package_names_loads_cleanly() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "app.py", "def main(): pass\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _sid = ingest(&svc, &src);

    // Find the store and delete package_names from meta
    let repos_dir = tmp.path().join("idx").join("repos");
    for entry in fs::read_dir(&repos_dir).unwrap() {
        let entry = entry.unwrap();
        let db_path = entry.path().join("index.sqlite3");
        if db_path.exists() {
            // Delete package_names from meta table
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let _ = conn.execute("DELETE FROM meta WHERE key = 'package_names'", []);
            drop(conn);

            // Reload and verify it doesn't crash
            let store = IndexStore::open(db_path).unwrap();
            let names: Vec<String> = store.get_package_names().unwrap();
            assert!(
                names.is_empty(),
                "expected empty package_names after deletion, got {names:?}"
            );
            return;
        }
    }
}

#[test]
fn case_package_names_persisted_via_save_index() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("test.sqlite3");
    let store = IndexStore::open(db_path).unwrap();

    store
        .set_package_names(&["my-package".to_string(), "my-alias".to_string()])
        .unwrap();

    let names: Vec<String> = store.get_package_names().unwrap();
    assert!(
        names.contains(&"my-package".to_string()),
        "names = {names:?}"
    );
    assert!(names.contains(&"my-alias".to_string()), "names = {names:?}");
}

// ============================================================
// Phase 4: build_package_registry
// ============================================================

#[test]
fn case_build_package_registry_maps_package_to_repo() {
    let repos = vec![RepoEntry {
        repo_id: "local/repo-a".to_string(),
        package_names: vec!["requests".to_string()],
    }];
    let registry = build_package_registry(&repos);
    assert!(registry["requests"].contains(&"local/repo-a".to_string()));
}

#[test]
fn case_build_package_registry_two_repos_same_package() {
    let repos = vec![
        RepoEntry {
            repo_id: "local/repo-a".to_string(),
            package_names: vec!["shared-pkg".to_string()],
        },
        RepoEntry {
            repo_id: "local/repo-b".to_string(),
            package_names: vec!["shared-pkg".to_string()],
        },
    ];
    let registry = build_package_registry(&repos);
    let entries = &registry["shared-pkg"];
    assert_eq!(entries.len(), 2, "expected 2 repos, got {entries:?}");
    assert!(entries.contains(&"local/repo-a".to_string()));
    assert!(entries.contains(&"local/repo-b".to_string()));
}

#[test]
fn case_find_repos_for_package_returns_correct_repos() {
    let repos = vec![RepoEntry {
        repo_id: "local/flask-repo".to_string(),
        package_names: vec!["flask".to_string()],
    }];
    let result = find_repos_for_package("flask", &repos);
    assert!(
        result.contains(&"local/flask-repo".to_string()),
        "result = {result:?}"
    );
}

#[test]
fn case_find_repos_for_nonexistent_package_returns_empty() {
    let repos = vec![RepoEntry {
        repo_id: "local/repo-a".to_string(),
        package_names: vec!["numpy".to_string()],
    }];
    let result = find_repos_for_package("pandas", &repos);
    assert!(result.is_empty(), "result = {result:?}");
}

// ============================================================
// Phase 5: find_importers cross_repo=False baseline
// ============================================================

#[test]
fn case_find_importers_cross_repo_false_finds_local_importers() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(
        &src,
        "main.py",
        "from utils import helper\nresult = helper()\n",
    );

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);

    // Use inspect_graph importers mode to find importers of utils.py
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    let importers = data["importers"].as_array().unwrap();
    let importer_strs: Vec<&str> = importers.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        importer_strs.contains(&"main.py"),
        "expected main.py in importers: {importers:?}"
    );
}

#[test]
fn case_find_importers_cross_repo_false_no_cross_repo_key() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(
        &src,
        "main.py",
        "from utils import helper\nresult = helper()\n",
    );

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "importers",
                "file_path": "utils.py",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    // cross_repo_importer_count should not be present in non-cross-repo mode
    assert!(
        data.get("cross_repo_importer_count").is_none(),
        "cross_repo_importer_count should not appear: {data:?}"
    );
}

// ============================================================
// Phase 5b: find_importers cross_repo=True
// ============================================================

#[test]
fn case_find_importers_cross_repo_true_finds_cross_repo_importers() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do_thing(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do_thing()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let provider_id = ingest(&svc, &provider);
    let _consumer_id = ingest(&svc, &consumer);

    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": provider_id,
                "graph": "importers",
                "file_path": "api.py",
                "cross_repo": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data.get("cross_repo_importer_count").is_some(),
        "cross_repo_importer_count should be present when cross_repo=true: {data:?}"
    );
    let count = data["cross_repo_importer_count"].as_u64().unwrap();
    assert!(
        count >= 1,
        "expected at least 1 cross-repo importer, got {count}"
    );
}

#[test]
fn case_find_importers_cross_repo_results_have_correct_fields() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do_thing(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do_thing()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let provider_id = ingest(&svc, &provider);
    let _consumer_id = ingest(&svc, &consumer);

    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": provider_id,
                "graph": "importers",
                "file_path": "api.py",
                "cross_repo": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let cross_importers = r["data"]["cross_repo_importers"].as_array().unwrap();
    for imp in cross_importers {
        assert!(imp.get("from_repo").is_some(), "missing from_repo: {imp:?}");
        assert!(imp.get("to_repo").is_some(), "missing to_repo: {imp:?}");
        assert!(
            imp.get("package_name").is_some(),
            "missing package_name: {imp:?}"
        );
    }
}

// ============================================================
// Phase 5c: get_blast_radius cross_repo
// ============================================================

#[test]
fn case_blast_radius_cross_repo_false_no_cross_repo_key() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(
        &src,
        "main.py",
        "from utils import helper\nresult = helper()\n",
    );

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "blast_radius",
                "target": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data.get("cross_repo_key").is_none(),
        "cross_repo_key should NOT appear when cross_repo is false/default: {data:?}"
    );
}

#[test]
fn case_blast_radius_cross_repo_true_adds_cross_repo_field() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(
        &src,
        "main.py",
        "from utils import helper\nresult = helper()\n",
    );

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "blast_radius",
                "target": "helper",
                "cross_repo": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data.get("cross_repo_key").is_some(),
        "cross_repo_key should appear when cross_repo=true: {data:?}"
    );
    let key = data["cross_repo_key"].as_str().unwrap();
    assert!(
        key.contains("helper"),
        "cross_repo_key should contain symbol name: {key}"
    );
}

// ============================================================
// Phase 5d: get_dependency_graph cross_repo
// ============================================================

#[test]
fn case_dependency_graph_cross_repo_false_no_cross_repo_edges() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(&src, "main.py", "from utils import helper\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": sid,
                "graph": "dependencies",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data.get("cross_repo_edges").is_none(),
        "cross_repo_edges should NOT appear when cross_repo is false/default: {data:?}"
    );
}

#[test]
fn case_dependency_graph_cross_repo_true_returns_dict() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do_thing(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do_thing()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _provider_id = ingest(&svc, &provider);
    let consumer_id = ingest(&svc, &consumer);

    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": consumer_id,
                "graph": "dependencies",
                "cross_repo": true,
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(
        data.get("cross_repo_edges").is_some(),
        "cross_repo_edges should be present when cross_repo=true: {data:?}"
    );
    let edges = data["cross_repo_edges"].as_array().unwrap();
    // Should have at least one cross-repo edge (consumer -> provider via mylib)
    assert!(
        !edges.is_empty(),
        "expected cross-repo edges, got empty: {data:?}"
    );
    for edge in edges {
        assert!(
            edge.get("from_repo").is_some(),
            "missing from_repo: {edge:?}"
        );
        assert!(edge.get("to_repo").is_some(), "missing to_repo: {edge:?}");
        assert!(
            edge.get("package_name").is_some(),
            "missing package_name: {edge:?}"
        );
    }
}

// ============================================================
// Phase 6: get_cross_repo_map
// ============================================================

#[test]
fn case_get_cross_repo_map_no_filter_returns_all() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let provider_id = ingest(&svc, &provider);
    let consumer_id = ingest(&svc, &consumer);

    let r = svc.dispatch("get_cross_repo_map", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    assert!(data.get("repos").is_some(), "missing repos: {data:?}");
    assert!(
        data.get("cross_repo_edges").is_some(),
        "missing cross_repo_edges: {data:?}"
    );

    let repos = data["repos"].as_array().unwrap();
    let repo_ids: Vec<&str> = repos.iter().filter_map(|r| r["repo"].as_str()).collect();
    assert!(
        repo_ids.contains(&provider_id.as_str()),
        "provider missing: {repo_ids:?}"
    );
    assert!(
        repo_ids.contains(&consumer_id.as_str()),
        "consumer missing: {repo_ids:?}"
    );
}

#[test]
fn case_get_cross_repo_map_with_filter() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do_thing(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do_thing()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _provider_id = ingest(&svc, &provider);
    let consumer_id = ingest(&svc, &consumer);

    let r = svc
        .dispatch("get_cross_repo_map", json!({ "source_id": consumer_id }))
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let data = &r["data"];
    let repos = data["repos"].as_array().unwrap();
    assert_eq!(repos.len(), 1, "expected 1 repo with filter: {repos:?}");
    assert_eq!(repos[0]["repo"].as_str().unwrap(), consumer_id);
}

#[test]
fn case_get_cross_repo_map_unknown_repo_returns_error() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "app.py", "print('hello')\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "get_cross_repo_map",
            json!({ "source_id": "nonexistent/repo" }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn case_get_cross_repo_map_cross_repo_edges_structure() {
    let tmp = tempdir().unwrap();
    let provider = tmp.path().join("provider");
    let consumer = tmp.path().join("consumer");
    fs::create_dir_all(&provider).unwrap();
    fs::create_dir_all(&consumer).unwrap();

    _write(&provider, "pyproject.toml", "[project]\nname = \"mylib\"\n");
    _write(&provider, "api.py", "def do_thing(): pass\n");
    _write(&consumer, "app.py", "import mylib\nmylib.do_thing()\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _provider_id = ingest(&svc, &provider);
    let _consumer_id = ingest(&svc, &consumer);

    let r = svc.dispatch("get_cross_repo_map", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    let edges = r["data"]["cross_repo_edges"].as_array().unwrap();
    for edge in edges {
        assert!(
            edge.get("from_repo").is_some(),
            "missing from_repo: {edge:?}"
        );
        assert!(edge.get("to_repo").is_some(), "missing to_repo: {edge:?}");
        assert!(
            edge.get("package_name").is_some(),
            "missing package_name: {edge:?}"
        );
    }
}

// ============================================================
// Phase 7: CODESIGHT_CROSS_REPO_DEFAULT env var
// ============================================================

#[test]
fn case_env_var_sets_cross_repo_default() {
    // Setting CODESIGHT_CROSS_REPO_DEFAULT=true enables cross-repo by default.
    // We test the AppConfig field directly since env vars are process-global.
    let tmp = tempdir().unwrap();
    let mut config = AppConfig::new(tmp.path().join("idx"));
    config.cross_repo_default = true;
    assert!(
        config.cross_repo_default,
        "cross_repo_default should be true"
    );

    // Verify it affects blast_radius output (cross_repo_key appears)
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(&src, "main.py", "from utils import helper\n");

    let svc = CodeService::new(config).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "blast_radius",
                "target": "helper",
                // No explicit cross_repo param — should use config default (true)
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"].get("cross_repo_key").is_some(),
        "cross_repo_key should appear when config.cross_repo_default=true: {:?}",
        r["data"]
    );
}

#[test]
fn case_env_var_false_keeps_default() {
    // CROSS_REPO_DEFAULT=false keeps cross-repo off by default.
    let tmp = tempdir().unwrap();
    let mut config = AppConfig::new(tmp.path().join("idx"));
    config.cross_repo_default = false;
    assert!(
        !config.cross_repo_default,
        "cross_repo_default should be false"
    );

    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def helper(): return 42\n");
    _write(&src, "main.py", "from utils import helper\n");

    let svc = CodeService::new(config).unwrap();
    let sid = ingest(&svc, &src);

    let r = svc
        .dispatch(
            "evaluate_impact",
            json!({
                "source_id": sid,
                "analysis": "blast_radius",
                "target": "helper",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(
        r["data"].get("cross_repo_key").is_none(),
        "cross_repo_key should NOT appear when config.cross_repo_default=false: {:?}",
        r["data"]
    );
}

// ============================================================
// Edge cases
// ============================================================

#[test]
fn case_no_cross_repo_match_returns_empty() {
    let tmp = tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    _write(&src, "utils.py", "def foo(): pass\n");
    _write(&src, "main.py", "from utils import foo\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _sid = ingest(&svc, &src);

    let r = svc.dispatch("get_cross_repo_map", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    // Single repo, no cross-repo edges expected (or empty list)
    let edges = r["data"]["cross_repo_edges"].as_array().unwrap();
    // Internal imports shouldn't create cross-repo edges
    assert!(
        edges.is_empty() || edges.iter().all(|e| { e["from_repo"] == e["to_repo"] }),
        "expected no cross-repo edges for single-repo: {edges:?}"
    );
}

#[test]
fn case_circular_cross_repo_dependency_no_infinite_loop() {
    let tmp = tempdir().unwrap();
    let src_a = tmp.path().join("a");
    let src_b = tmp.path().join("b");
    fs::create_dir_all(&src_a).unwrap();
    fs::create_dir_all(&src_b).unwrap();

    _write(&src_a, "pyproject.toml", "[project]\nname = \"pkg-a\"\n");
    _write(&src_a, "__init__.py", "import pkg_b\n");
    _write(&src_b, "pyproject.toml", "[project]\nname = \"pkg-b\"\n");
    _write(&src_b, "__init__.py", "import pkg_a\n");

    let svc = CodeService::new(AppConfig::new(tmp.path().join("idx"))).unwrap();
    let _repo_a = ingest(&svc, &src_a);
    let _repo_b = ingest(&svc, &src_b);

    // Must complete without hanging
    let r = svc.dispatch("get_cross_repo_map", json!({})).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    assert!(r["data"].get("repos").is_some());
}

#[test]
fn case_extract_package_names_nonexistent_dir_returns_empty() {
    let names = extract_package_names("/nonexistent/path/that/does/not/exist");
    assert!(names.is_empty());
}
