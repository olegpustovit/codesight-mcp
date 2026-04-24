//! Integration tests: json.

use std::collections::HashSet;
use std::path::Path;

use codesight_core::SymbolKind;
use codesight_language_support::LanguageRegistry;
use codesight_language_support::scm_engine::{ScmExtractor, parse_scm_header, query_files_slice};

fn scm_extractor(lang_key: &str) -> ScmExtractor {
    let source = query_files_slice()
        .iter()
        .find(|(k, _)| *k == lang_key)
        .unwrap_or_else(|| panic!("no query for {lang_key}"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).unwrap()
}

// ---------------------------------------------------------------------------
// Inline fixture (matches reference `_PACKAGE_JSON`)
// ---------------------------------------------------------------------------

const PACKAGE_JSON: &str = r#"{
  "name": "myapp",
  "version": "1.0.0",
  "description": "A sample app",
  "dependencies": {
    "react": "^18.0.0",
    "axios": "^1.0.0"
  },
  "devDependencies": {
    "vite": "^4.0.0"
  },
  "scripts": {
    "build": "vite build",
    "dev": "vite"
  }
}"#;

/// Helper: extract symbols from `PACKAGE_JSON` via `JsonExtractor`.
fn syms() -> Vec<codesight_core::StoredSymbol> {
    scm_extractor("json")
        .extract("package.json", PACKAGE_JSON)
        .expect("JsonExtractor should parse the package.json fixture")
}

// ---------------------------------------------------------------------------
// Extension / language detection
// ---------------------------------------------------------------------------
/// `get_language_for_path("package.json") == "json"`
#[test]
fn test_json_extension_detected() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("package.json"));
    assert!(ext.is_some(), "registry should find an extractor for .json");
    assert_eq!(ext.unwrap().id().as_str(), "json");
}
/// `".json" in LANGUAGE_EXTENSIONS` and maps to `"json"`.
#[test]
fn test_json_extension_in_registry() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("anything.json"));
    assert!(ext.is_some(), ".json must be registered");
    assert_eq!(ext.unwrap().id().as_str(), "json");
}
/// `api.openapi.json` and `api.swagger.json` should resolve to `"openapi"`,
/// not `"json"`.  The current registry does not support compound extensions
/// or an openapi extractor.
#[test]
fn test_openapi_json_not_overridden() {
    let reg = LanguageRegistry::default();

    let openapi = reg.extractor_for_path(Path::new("api.openapi.json"));
    assert!(openapi.is_some());
    assert_eq!(openapi.unwrap().id().as_str(), "openapi");

    let swagger = reg.extractor_for_path(Path::new("api.swagger.json"));
    assert!(swagger.is_some());
    assert_eq!(swagger.unwrap().id().as_str(), "openapi");
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------
/// `len(_syms()) >= 6`
#[test]
fn test_json_returns_symbols() {
    let s = syms();
    assert!(s.len() >= 6, "expected at least 6 symbols, got {}", s.len());
}
/// Top-level keys `name`, `version`, `dependencies`, `scripts` are present.
#[test]
fn test_json_top_level_keys_extracted() {
    let symbols = syms();
    let names: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("name"), "missing 'name'");
    assert!(names.contains("version"), "missing 'version'");
    assert!(names.contains("dependencies"), "missing 'dependencies'");
    assert!(names.contains("scripts"), "missing 'scripts'");
}
/// Every symbol has `kind == "constant"` and `language == "json"`.
#[test]
fn test_json_symbol_kind() {
    for s in &syms() {
        assert_eq!(
            s.kind,
            SymbolKind::Constant,
            "symbol '{}' should be Constant, got {:?}",
            s.name,
            s.kind
        );
        assert_eq!(
            s.language.as_str(),
            "json",
            "symbol '{}' language should be 'json', got '{}'",
            s.name,
            s.language
        );
    }
}
/// The `"name"` symbol has `line >= 1`.
#[test]
fn test_json_symbol_line() {
    let s = syms()
        .into_iter()
        .find(|s| s.name == "name")
        .expect("should have a 'name' symbol");
    assert!(
        s.start_line >= 1,
        "start_line should be >= 1, got {}",
        s.start_line
    );
}
/// The `"name"` symbol's signature contains `"myapp"`.
#[test]
fn test_json_symbol_signature() {
    let s = syms()
        .into_iter()
        .find(|s| s.name == "name")
        .expect("should have a 'name' symbol");
    assert!(
        s.signature.contains("myapp"),
        "signature should contain 'myapp', got: {}",
        s.signature
    );
}
/// All symbol IDs are distinct.
#[test]
fn test_json_symbol_ids_unique() {
    let symbols = syms();
    let ids: HashSet<&str> = symbols.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids.len(),
        symbols.len(),
        "duplicate IDs detected among {} symbols",
        symbols.len()
    );
}
/// Every symbol has `byte_offset >= 0`, `byte_length > 0`, and a non-empty
/// `content_hash`.
///
/// Mapped fields: `byte_offset` -> `start_byte`, `byte_length` -> `end_byte - start_byte`,
/// `content_hash` -> `source_hash`.
#[test]
fn test_json_symbol_has_byte_info() {
    for s in &syms() {
        // start_byte is usize, always >= 0
        let byte_length = s.end_byte - s.start_byte;
        assert!(
            byte_length > 0,
            "symbol '{}' should have byte_length > 0",
            s.name
        );
        assert!(
            !s.source_hash.is_empty(),
            "symbol '{}' should have a non-empty source_hash",
            s.name
        );
    }
}
/// Parsing through the registry-level `parse_file` path yields the same
/// key symbols.
#[test]
fn test_json_via_parse_file() {
    let reg = LanguageRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("package.json"))
        .expect("registry should resolve package.json");
    let symbols = ext
        .extract_symbols("package.json", PACKAGE_JSON)
        .expect("extraction should succeed");
    let names: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("name"), "missing 'name' via registry path");
    assert!(
        names.contains("dependencies"),
        "missing 'dependencies' via registry path"
    );
}
/// `_parse_json_symbols(b"{}", "empty.json") == []`
#[test]
fn test_json_empty_object() {
    let s = scm_extractor("json")
        .extract("empty.json", "{}")
        .expect("should parse empty object");
    assert!(s.is_empty(), "empty object should yield no symbols");
}
/// `_parse_json_symbols(b"", "empty.json") == []`
#[test]
fn test_json_empty_file() {
    let s = scm_extractor("json")
        .extract("empty.json", "")
        .expect("should handle empty file");
    assert!(s.is_empty(), "empty file should yield no symbols");
}
/// An array at the root produces no symbols.
#[test]
fn test_json_array_root() {
    let s = scm_extractor("json")
        .extract("list.json", r#"["a", "b", "c"]"#)
        .expect("should parse array root");
    assert!(s.is_empty(), "array root should yield no symbols");
}
/// Only root-level keys are extracted; nested keys like `react` and `build`
/// must not appear.
#[test]
fn test_json_no_nested_keys() {
    let symbols = syms();
    let names: HashSet<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        !names.contains("react"),
        "'react' is a nested key and should not be extracted"
    );
    assert!(
        !names.contains("build"),
        "'build' is a nested key and should not be extracted"
    );
}
