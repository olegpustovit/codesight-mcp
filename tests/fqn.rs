//! Integration tests: fqn.

use std::collections::{HashMap, HashSet};

use codesight_analysis::graph::{Psr4Map, fqn_to_symbol, symbol_to_fqn};

fn standard_psr4_map() -> Psr4Map {
    let mut map = HashMap::new();
    map.insert("App\\".to_string(), "app/".to_string());
    map.insert("Database\\".to_string(), "database/".to_string());
    map
}

fn standard_source_files() -> HashSet<String> {
    [
        "app/Models/User.php",
        "app/Models/Post.php",
        "app/Http/Controllers/UserController.php",
        "app/Http/Controllers/Admin/UserController.php",
        "app/helpers.php",
        "database/Seeders/UserSeeder.php",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

// ===========================================================================
// TestSymbolToFqn (8 tests)
// ===========================================================================

#[test]
fn symbol_to_fqn_class_symbol() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("app/Models/User.php::User#class", &map);
    assert_eq!(result, Some("App\\Models\\User".to_string()));
}

#[test]
fn symbol_to_fqn_nested_namespace() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn(
        "app/Http/Controllers/Admin/UserController.php::UserController#class",
        &map,
    );
    assert_eq!(
        result,
        Some("App\\Http\\Controllers\\Admin\\UserController".to_string())
    );
}

#[test]
fn symbol_to_fqn_method_symbol() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("app/Models/User.php::User.posts#method", &map);
    assert_eq!(result, Some("App\\Models\\User::posts".to_string()));
}

#[test]
fn symbol_to_fqn_non_php_returns_none() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("src/main.py::main#function", &map);
    assert_eq!(result, None, "non-PHP files should return None");
}

#[test]
fn symbol_to_fqn_no_matching_prefix_returns_none() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("vendor/Foo.php::Foo#class", &map);
    assert_eq!(result, None, "no PSR-4 prefix matches vendor/");
}

#[test]
fn symbol_to_fqn_no_separator_returns_none() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("just-a-string", &map);
    assert_eq!(result, None, "no :: separator should return None");
}

#[test]
fn symbol_to_fqn_database_prefix() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("database/Seeders/UserSeeder.php::UserSeeder#class", &map);
    assert_eq!(result, Some("Database\\Seeders\\UserSeeder".to_string()));
}

#[test]
fn symbol_to_fqn_function_kind_returns_none() {
    let map = standard_psr4_map();
    let result = symbol_to_fqn("app/helpers.php::formatDate#function", &map);
    assert_eq!(result, None, "function symbols should not have FQNs");
}

// ===========================================================================
// TestFqnToSymbol (11 tests)
// ===========================================================================

#[test]
fn fqn_to_symbol_simple_class() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("App\\Models\\User", &map, &files);
    assert_eq!(result, Some("app/Models/User.php::User#class".to_string()));
}

#[test]
fn fqn_to_symbol_nested_class() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("App\\Http\\Controllers\\UserController", &map, &files);
    assert_eq!(
        result,
        Some("app/Http/Controllers/UserController.php::UserController#class".to_string())
    );
}

#[test]
fn fqn_to_symbol_with_method() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("App\\Models\\User::posts", &map, &files);
    assert_eq!(
        result,
        Some("app/Models/User.php::User.posts#method".to_string())
    );
}

#[test]
fn fqn_to_symbol_missing_file_returns_none() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("App\\Models\\Order", &map, &files);
    assert_eq!(result, None, "Order.php not in source files");
}

#[test]
fn fqn_to_symbol_database_prefix() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("Database\\Seeders\\UserSeeder", &map, &files);
    assert_eq!(
        result,
        Some("database/Seeders/UserSeeder.php::UserSeeder#class".to_string())
    );
}

#[test]
fn fqn_to_symbol_roundtrip_class() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let symbol_id = "app/Models/Post.php::Post#class";
    let fqn = symbol_to_fqn(symbol_id, &map).unwrap();
    let back = fqn_to_symbol(&fqn, &map, &files).unwrap();
    assert_eq!(back, symbol_id);
}

#[test]
fn fqn_to_symbol_roundtrip_method() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let symbol_id = "app/Models/User.php::User.posts#method";
    let fqn = symbol_to_fqn(symbol_id, &map).unwrap();
    let back = fqn_to_symbol(&fqn, &map, &files).unwrap();
    assert_eq!(back, symbol_id);
}

#[test]
fn fqn_to_symbol_empty_psr4_map() {
    let map: Psr4Map = HashMap::new();
    let files = standard_source_files();
    let result = fqn_to_symbol("App\\Models\\User", &map, &files);
    assert_eq!(result, None);
}

#[test]
fn fqn_to_symbol_empty_source_files() {
    let map = standard_psr4_map();
    let files: HashSet<String> = HashSet::new();
    let result = fqn_to_symbol("App\\Models\\User", &map, &files);
    assert_eq!(result, None);
}

#[test]
fn fqn_to_symbol_without_backslash() {
    let map = standard_psr4_map();
    let files = standard_source_files();
    let result = fqn_to_symbol("User", &map, &files);
    assert_eq!(
        result, None,
        "simple name without namespace should not resolve"
    );
}

// ===========================================================================
// TestResolveFqnIntegration (4 tests)
// ===========================================================================

use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

fn write_file(base: &Path, rel: &str, content: &str) {
    let p = base.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, content).unwrap();
}

fn index_code(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn resolve_fqn_call(svc: &CodeService, sid: &str, fqn: &str) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "resolve_fqn",
            "fqn": fqn,
        }),
    )
    .unwrap()
}
#[test]
fn resolve_fqn_indexed_class() {
    let ws = tempdir().unwrap();
    let root = ws.path();
    write_file(root, "artisan", "<?php\n");
    write_file(
        root,
        "composer.json",
        &serde_json::to_string(&json!({
            "require": {"laravel/framework": "^11.0"},
            "autoload": {"psr-4": {"App\\": "app/"}},
        }))
        .unwrap(),
    );
    write_file(
        root,
        "app/Models/User.php",
        "<?php\nnamespace App\\Models;\nclass User {}\n",
    );

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, root);
    let r = resolve_fqn_call(&svc, &sid, "App\\Models\\User");

    assert!(r["ok"].as_bool().unwrap(), "resolve_fqn failed: {r:?}");
    assert_eq!(
        r["data"]["resolved"].as_str().unwrap(),
        "app/Models/User.php::User#class"
    );
}
#[test]
fn resolve_fqn_unresolvable() {
    let ws = tempdir().unwrap();
    let root = ws.path();
    write_file(
        root,
        "composer.json",
        &serde_json::to_string(&json!({
            "autoload": {"psr-4": {"App\\": "app/"}},
        }))
        .unwrap(),
    );
    write_file(root, "app/Foo.php", "<?php\nclass Foo {}\n");

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, root);
    let r = resolve_fqn_call(&svc, &sid, "App\\Models\\Missing");

    assert!(
        !r["ok"].as_bool().unwrap(),
        "should fail for missing FQN: {r:?}"
    );
    let msg = r["error"]["message"].as_str().unwrap().to_lowercase();
    assert!(
        msg.contains("could not be resolved"),
        "error should mention 'could not be resolved', got: {msg}"
    );
}
#[test]
fn resolve_fqn_no_psr4_returns_error() {
    let ws = tempdir().unwrap();
    let root = ws.path();
    write_file(root, "main.py", "print('hello')\n");

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, root);
    let r = resolve_fqn_call(&svc, &sid, "App\\Models\\User");

    assert!(
        !r["ok"].as_bool().unwrap(),
        "should fail without PSR-4: {r:?}"
    );
    let msg = r["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("PSR-4"),
        "error should mention 'PSR-4', got: {msg}"
    );
}
#[test]
fn resolve_fqn_repo_not_found() {
    let ws = tempdir().unwrap();
    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "nonexistent/repo",
                "graph": "resolve_fqn",
                "fqn": "App\\Models\\User",
            }),
        )
        .unwrap();

    assert!(
        !r["ok"].as_bool().unwrap(),
        "should fail for unknown repo: {r:?}"
    );
    let msg = r["error"]["message"].as_str().unwrap().to_lowercase();
    assert!(
        msg.contains("not indexed")
            || msg.contains("not found")
            || msg.contains("not been indexed"),
        "error should mention 'not indexed' or 'not found', got: {msg}"
    );
}
