//! Integration tests: razor.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::Path;

use codesight_core::{StoredSymbol, SymbolKind};
use codesight_language_support::LanguageRegistry;
use codesight_language_support::languages::razor::extractor::RazorExtractor;

use super::support::code_reference_fixtures_dir;

fn fixture_source(name: &str) -> String {
    let path = code_reference_fixtures_dir().join("razor").join(name);
    std::fs::read_to_string(path).expect("fixture must exist")
}

fn extract_cshtml_symbols() -> Vec<StoredSymbol> {
    let reg = LanguageRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("Views/Profile/Index.cshtml"))
        .expect("razor extractor");
    ext.extract_symbols(
        "Views/Profile/Index.cshtml",
        &fixture_source("sample.cshtml"),
    )
    .expect("extraction should succeed")
}

fn extract_blazor_symbols() -> Vec<StoredSymbol> {
    let reg = LanguageRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("Pages/Counter.razor"))
        .expect("razor extractor");
    ext.extract_symbols("Pages/Counter.razor", &fixture_source("Counter.razor"))
        .expect("extraction should succeed")
}

fn extract_layout_symbols() -> Vec<StoredSymbol> {
    let reg = LanguageRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("Views/Shared/_Layout.cshtml"))
        .expect("razor extractor");
    ext.extract_symbols(
        "Views/Shared/_Layout.cshtml",
        &fixture_source("dotnet_aspnetcore_layout.cshtml"),
    )
    .expect("extraction should succeed")
}

// ---------------------------------------------------------------------------
// Extension / language detection
// ---------------------------------------------------------------------------

#[test]
fn test_cshtml_extension_detected() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("Views/Profile/Index.cshtml"));
    assert!(ext.is_some(), ".cshtml should be detected");
    assert_eq!(ext.unwrap().id().as_str(), "razor");
}

#[test]
fn test_cshtml_extension_in_registry() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("test.cshtml"));
    assert!(ext.is_some(), ".cshtml must be registered");
    assert_eq!(ext.unwrap().id().as_str(), "razor");
}

#[test]
fn test_parse_razor_mixed_symbols() {
    let syms = extract_cshtml_symbols();

    // Index class (view name)
    let view = syms
        .iter()
        .find(|s| s.name == "Index" && s.kind == SymbolKind::Class);
    assert!(view.is_some(), "Index class should exist");
    assert_eq!(view.unwrap().language.as_str(), "razor");

    // FullName method (C#)
    let fullname = syms.iter().find(|s| s.name == "FullName");
    assert!(fullname.is_some(), "FullName should exist");
    assert_eq!(fullname.unwrap().kind, SymbolKind::Method);
    assert_eq!(fullname.unwrap().language.as_str(), "csharp");
    assert_eq!(fullname.unwrap().qualified_name, "Index.FullName");

    // Count constant (C#)
    let count = syms.iter().find(|s| s.name == "Count");
    assert!(count.is_some(), "Count should exist");
    assert_eq!(count.unwrap().kind, SymbolKind::Constant);
    assert_eq!(count.unwrap().language.as_str(), "csharp");

    // profile-card HTML id
    let profile_card = syms.iter().find(|s| s.name == "profile-card");
    assert!(profile_card.is_some(), "profile-card should exist");
    assert_eq!(profile_card.unwrap().kind, SymbolKind::Constant);
    assert_eq!(profile_card.unwrap().language.as_str(), "razor");

    // profile.js external script
    let profile_js = syms.iter().find(|s| s.name == "profile.js");
    assert!(profile_js.is_some(), "profile.js should exist");
    assert_eq!(profile_js.unwrap().kind, SymbolKind::Function);
    assert_eq!(profile_js.unwrap().language.as_str(), "razor");

    // toggleCard inline JS function
    let toggle = syms.iter().find(|s| s.name == "toggleCard");
    assert!(toggle.is_some(), "toggleCard should exist");
    assert_eq!(toggle.unwrap().kind, SymbolKind::Function);
    assert_eq!(toggle.unwrap().language.as_str(), "javascript");
    assert_eq!(toggle.unwrap().qualified_name, "Index.toggleCard");

    // style_1 (style block)
    let style = syms.iter().find(|s| s.name == "style_1");
    assert!(style.is_some(), "style_1 should exist");
    assert_eq!(style.unwrap().kind, SymbolKind::Constant);
    assert_eq!(style.unwrap().language.as_str(), "razor");
}

#[test]
fn test_parse_razor_code_block_handles_nested_braces_and_strings() {
    let source = r#"@code {
    public string Render()
    {
        var json = "{ \"enabled\": true }";
        if (json.Contains("{"))
        {
            return json;
        }
        return string.Empty;
    }
}
"#;
    let reg = LanguageRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("Views/Shared/Editor.cshtml"))
        .expect("razor extractor");
    let symbols = ext
        .extract_symbols("Views/Shared/Editor.cshtml", source)
        .unwrap();

    let view = symbols
        .iter()
        .find(|s| s.name == "Editor" && s.kind == SymbolKind::Class);
    assert!(view.is_some(), "Editor class should exist");

    let render = symbols.iter().find(|s| s.name == "Render");
    assert!(render.is_some(), "Render method should exist");
    assert_eq!(render.unwrap().kind, SymbolKind::Method);
    assert_eq!(render.unwrap().language.as_str(), "csharp");
    assert!(
        render.unwrap().start_line >= 2,
        "Render should be on line >= 2"
    );
    assert!(
        render.unwrap().end_line >= render.unwrap().start_line,
        "end_line >= start_line"
    );
}

#[test]
fn test_discover_local_files_includes_cshtml() {
    // This tests that the LanguageRegistry recognizes .cshtml files
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("Views/Home/Index.cshtml"));
    assert!(ext.is_some(), ".cshtml should be recognized");
}

#[test]
fn test_index_folder_parses_razor_symbols() {
    let syms = extract_cshtml_symbols();
    assert!(
        syms.len() >= 6,
        "expected >= 6 symbols from sample.cshtml, got {}",
        syms.len()
    );
}

#[test]
fn test_parse_public_razor_layout_fixture() {
    let syms = extract_layout_symbols();

    let view = syms
        .iter()
        .find(|s| s.name == "_Layout" && s.kind == SymbolKind::Class);
    assert!(view.is_some(), "_Layout class should exist");
    assert_eq!(view.unwrap().language.as_str(), "razor");

    let scripts: FxHashMap<_, _> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .map(|s| (s.name.as_str(), s))
        .collect();
    assert!(
        scripts.contains_key("jquery.min.js"),
        "missing jquery.min.js"
    );
    assert!(
        scripts.contains_key("bootstrap.bundle.min.js"),
        "missing bootstrap.bundle.min.js"
    );
    assert!(scripts.contains_key("site.js"), "missing site.js");
}

// ---------------------------------------------------------------------------
// .razor (Blazor) tests
// ---------------------------------------------------------------------------

#[test]
fn test_razor_extension_detected() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("Pages/Counter.razor"));
    assert!(ext.is_some(), ".razor should be detected");
    assert_eq!(ext.unwrap().id().as_str(), "razor");
}

#[test]
fn test_razor_extension_in_registry() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("test.razor"));
    assert!(ext.is_some(), ".razor must be registered");
    assert_eq!(ext.unwrap().id().as_str(), "razor");
}

#[test]
fn test_blazor_component_symbol() {
    let syms = extract_blazor_symbols();
    let component = syms
        .iter()
        .find(|s| s.name == "Counter" && s.kind == SymbolKind::Class);
    assert!(component.is_some(), "Counter class should exist");
    assert_eq!(component.unwrap().language.as_str(), "razor");
    assert_eq!(component.unwrap().start_line, 1);
}

#[test]
fn test_blazor_page_routes() {
    let syms = extract_blazor_symbols();
    let routes: FxHashMap<_, _> = syms
        .iter()
        .filter(|s| s.signature.starts_with("@page "))
        .map(|s| (s.name.as_str(), s))
        .collect();
    assert!(routes.contains_key("/counter"), "missing /counter route");
    assert!(
        routes.contains_key("/counter/{StartingCount:int}"),
        "missing parameterized route"
    );
    assert_eq!(routes["/counter"].kind, SymbolKind::Constant);
    assert_eq!(routes["/counter"].language.as_str(), "razor");
}

#[test]
fn test_blazor_inject_directives() {
    let syms = extract_blazor_symbols();
    let injects: FxHashMap<_, _> = syms
        .iter()
        .filter(|s| s.signature.starts_with("@inject "))
        .map(|s| (s.name.as_str(), s))
        .collect();
    assert!(injects.contains_key("Logger"), "missing Logger inject");
    assert!(
        injects.contains_key("Navigation"),
        "missing Navigation inject"
    );
    assert_eq!(
        injects["Logger"].signature,
        "@inject ILogger<Counter> Logger"
    );
    assert_eq!(
        injects["Navigation"].signature,
        "@inject NavigationManager Navigation"
    );
    assert_eq!(injects["Logger"].kind, SymbolKind::Constant);
}

#[test]
fn test_blazor_code_block_csharp_members() {
    let syms = extract_blazor_symbols();

    let on_init = syms.iter().find(|s| s.name == "OnInitialized");
    assert!(on_init.is_some(), "OnInitialized should exist");
    assert_eq!(on_init.unwrap().language.as_str(), "csharp");

    let increment = syms.iter().find(|s| s.name == "IncrementCount");
    assert!(increment.is_some(), "IncrementCount should exist");
    assert_eq!(increment.unwrap().language.as_str(), "csharp");
}

/// Nested generics break the legacy regex return-type arm (`Task<…>` uses `[^>]+`).
#[test]
fn test_razor_code_block_nested_generic_method_via_tree_sitter() {
    let src = r#"@page "/x"
@code {
    public async Task<Dictionary<string, object>> FetchAllAsync()
    {
        return await Task.FromResult(new Dictionary<string, object>());
    }
}
"#;
    let syms = RazorExtractor
        .extract("Pages/Data.razor", src)
        .expect("extract");
    let m = syms
        .iter()
        .find(|s| s.name == "FetchAllAsync" && s.language.as_str() == "csharp");
    assert!(
        m.is_some(),
        "expected csharp.FetchAllAsync from tree-sitter, got csharp names: {:?}",
        syms.iter()
            .filter(|s| s.language.as_str() == "csharp")
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_blazor_html_ids() {
    let syms = extract_blazor_symbols();
    let ids: FxHashSet<_> = syms
        .iter()
        .filter(|s| {
            s.kind == SymbolKind::Constant
                && s.language.as_str() == "razor"
                && !s.signature.starts_with("@page")
                && !s.signature.starts_with("@inject")
        })
        .map(|s| s.name.as_str())
        .collect();
    assert!(ids.contains("page-title"), "missing page-title");
    assert!(ids.contains("current-count"), "missing current-count");
    assert!(ids.contains("increment-btn"), "missing increment-btn");
}

#[test]
fn test_blazor_style_block() {
    let syms = extract_blazor_symbols();
    let style = syms.iter().find(|s| s.name == "style_1");
    assert!(style.is_some(), "style_1 should exist");
    assert_eq!(style.unwrap().kind, SymbolKind::Constant);
    assert_eq!(style.unwrap().language.as_str(), "razor");
}

#[test]
fn test_discover_local_files_includes_razor() {
    let reg = LanguageRegistry::default();
    let ext = reg.extractor_for_path(Path::new("Pages/Counter.razor"));
    assert!(ext.is_some(), ".razor should be recognized");
}

#[test]
fn test_index_folder_parses_blazor_symbols() {
    let syms = extract_blazor_symbols();
    assert!(
        syms.len() >= 5,
        "expected >= 5 blazor symbols, got {}",
        syms.len()
    );
}
