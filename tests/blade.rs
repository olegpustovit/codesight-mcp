//! Integration tests: blade.

use std::path::Path;

use codesight_mcp::extractors::ExtractorRegistry;
use codesight_mcp::model::StoredSymbol;
use codesight_mcp::test_support::code_reference_fixtures_dir;

fn fixture_source() -> String {
    let path = code_reference_fixtures_dir().join("blade/sample.blade.php");
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn extract_symbols() -> Vec<StoredSymbol> {
    let reg = ExtractorRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("resources/views/profile.blade.php"))
        .expect("blade extractor");
    ext.extract("resources/views/profile.blade.php", &fixture_source())
        .expect("extraction should succeed")
}

// ---------------------------------------------------------------------------
// Extension / language detection
// ---------------------------------------------------------------------------

#[test]
fn test_blade_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("resources/views/home.blade.php"));
    assert!(ext.is_some(), ".blade.php should be detected");
    assert_eq!(ext.unwrap().language_name(), "blade");
}

#[test]
fn test_blade_not_confused_with_php() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("app/Http/Controllers/HomeController.php"));
    assert!(ext.is_some(), ".php should be detected");
    assert_eq!(ext.unwrap().language_name(), "php");
}

#[test]
fn test_blade_extension_in_registry() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("test.blade.php"));
    assert!(ext.is_some(), ".blade.php must be registered");
    assert_eq!(ext.unwrap().language_name(), "blade");
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------

#[test]
fn test_blade_returns_symbols() {
    let syms = extract_symbols();
    assert!(syms.len() >= 5, "expected >= 5 symbols, got {}", syms.len());
}

#[test]
fn test_blade_extends_extracted() {
    let syms = extract_symbols();
    let extends: Vec<_> = syms
        .iter()
        .filter(|s| s.name == "layouts.app" && s.kind == codesight_mcp::model::SymbolKind::Type)
        .collect();
    assert!(!extends.is_empty(), "expected layouts.app type symbol");
}

#[test]
fn test_blade_sections_extracted() {
    let syms = extract_symbols();
    let section_names: std::collections::HashSet<_> = syms
        .iter()
        .filter(|s| s.kind == codesight_mcp::model::SymbolKind::Method)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        section_names.contains("content"),
        "missing section 'content'"
    );
    assert!(
        section_names.contains("sidebar"),
        "missing section 'sidebar'"
    );
}

#[test]
fn test_blade_component_extracted() {
    let syms = extract_symbols();
    let components: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == codesight_mcp::model::SymbolKind::Class && s.name == "alert")
        .collect();
    assert!(!components.is_empty(), "missing component 'alert'");
}

#[test]
fn test_blade_include_extracted() {
    let syms = extract_symbols();
    let includes: std::collections::HashSet<_> = syms
        .iter()
        .filter(|s| s.kind == codesight_mcp::model::SymbolKind::Function)
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        includes.contains("partials.avatar"),
        "missing include 'partials.avatar'"
    );
    assert!(
        includes.contains("partials.nav"),
        "missing include 'partials.nav'"
    );
}

#[test]
fn test_blade_push_extracted() {
    let syms = extract_symbols();
    let pushes: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == codesight_mcp::model::SymbolKind::Constant && s.name == "scripts")
        .collect();
    assert!(
        pushes.len() >= 1,
        "expected at least one 'scripts' constant"
    );
}

#[test]
fn test_blade_livewire_extracted() {
    let syms = extract_symbols();
    let livewire: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == codesight_mcp::model::SymbolKind::Class && s.name == "user-stats")
        .collect();
    assert_eq!(
        livewire.len(),
        1,
        "expected exactly one livewire 'user-stats'"
    );
}

#[test]
fn test_blade_symbols_have_line_numbers() {
    let syms = extract_symbols();
    for s in &syms {
        assert!(s.start_line >= 1, "symbol {} has line < 1", s.name);
    }
}

#[test]
fn test_blade_symbols_sorted_by_line() {
    let syms = extract_symbols();
    let lines: Vec<u32> = syms.iter().map(|s| s.start_line).collect();
    for w in lines.windows(2) {
        assert!(
            w[0] <= w[1],
            "symbols not sorted by line: {} > {}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn test_blade_symbol_ids_unique() {
    let syms = extract_symbols();
    let ids: std::collections::HashSet<_> = syms.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), syms.len(), "duplicate symbol IDs found");
}

#[test]
fn test_blade_language_field() {
    let syms = extract_symbols();
    for s in &syms {
        assert_eq!(
            s.language, "blade",
            "symbol {} has language {}",
            s.name, s.language
        );
    }
}
