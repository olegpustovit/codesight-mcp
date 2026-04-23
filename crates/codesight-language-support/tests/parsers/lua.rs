//! Integration tests: lua.

use std::path::Path;

use codesight_language_support::scm_engine::{ScmExtractor, parse_scm_header, query_files_slice};
use codesight_language_support::LanguageRegistry;
use codesight_core::SymbolKind;

fn scm_extractor(lang_key: &str) -> ScmExtractor {
    let source = query_files_slice()
        .iter()
        .find(|(k, _)| *k == lang_key)
        .unwrap_or_else(|| panic!("no query for {lang_key}"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).unwrap()
}

const LUA_SOURCE: &str = "\
--- Initialise the addon
-- @param name string
local function init(name)
    return {name = name}
end

function MyAddon.OnLoad(self)
    print(\"loaded\")
end

--- Handle combat log event
function MyAddon:OnCombatLogEvent(event, ...)
    self:process(event)
end
";

fn lua_symbols() -> Vec<codesight_core::StoredSymbol> {
    scm_extractor("lua")
        .extract("addon.lua", LUA_SOURCE)
        .expect("lua extraction failed")
}
///
/// Reference asserts:
///   - "init" is found by qualified_name
///   - kind == "function"
///   - parent is None
///   - docstring contains "Initialise the addon"
#[test]
fn test_lua_local_function() {
    let symbols = lua_symbols();
    let names: Vec<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    let sym = symbols
        .iter()
        .find(|s| s.qualified_name == "init")
        .unwrap_or_else(|| panic!("expected 'init' in qualified names: {names:?}"));
    assert_eq!(sym.kind, SymbolKind::Function);
    // parent is None => qualified_name has no dot separator (no scopes)
    assert!(
        !sym.qualified_name.contains('.'),
        "init should have no parent scope, got qualified_name={:?}",
        sym.qualified_name
    );
    assert!(
        sym.summary.contains("Initialise the addon"),
        "docstring should contain 'Initialise the addon', got: {:?}",
        sym.summary
    );
}
///
/// Reference asserts:
///   - qualified_name == "MyAddon.OnLoad"
///   - kind == "method"
///   - parent == "MyAddon"
///   - name == "OnLoad"
#[test]
fn test_lua_dot_method() {
    let symbols = lua_symbols();
    let sym = symbols
        .iter()
        .find(|s| s.qualified_name == "MyAddon.OnLoad")
        .unwrap_or_else(|| {
            let names: Vec<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
            panic!("expected 'MyAddon.OnLoad' in qualified names: {names:?}")
        });
    assert_eq!(sym.kind, SymbolKind::Method);
    // parent == "MyAddon" is encoded as the scope prefix in qualified_name
    assert!(sym.qualified_name.starts_with("MyAddon."));
    assert_eq!(sym.name, "OnLoad");
}
///
/// Reference asserts:
///   - qualified_name == "MyAddon:OnCombatLogEvent"
///   - kind == "method"
///   - parent == "MyAddon"
///   - docstring contains "Handle combat log event"
///
/// NOTE: The reference expects qualified_name with a colon separator
/// ("MyAddon:OnCombatLogEvent"), but the Rust extractor's `qualify()` helper
/// always joins scopes with '.', producing "MyAddon.OnCombatLogEvent".
/// The test asserts reference behavior; if the Rust output differs, it is
/// marked #[ignore].
#[test]
fn test_lua_colon_method_qualified_name_uses_colon() {
    let symbols = lua_symbols();
    let sym = symbols
        .iter()
        .find(|s| s.qualified_name == "MyAddon:OnCombatLogEvent")
        .unwrap_or_else(|| {
            let names: Vec<&str> = symbols.iter().map(|s| s.qualified_name.as_str()).collect();
            panic!("expected 'MyAddon:OnCombatLogEvent' in qualified names: {names:?}")
        });
    assert_eq!(sym.kind, SymbolKind::Method);
    assert_eq!(sym.name, "OnCombatLogEvent");
}

/// Non-ignored portion of test_lua_colon_method: kind, parent, docstring.
/// These hold regardless of the colon-vs-dot qualified_name difference.
#[test]
fn test_lua_colon_method_kind_parent_docstring() {
    let symbols = lua_symbols();
    // Find via name since qualified_name uses '.' in Rust
    let sym = symbols
        .iter()
        .find(|s| s.name == "OnCombatLogEvent")
        .unwrap_or_else(|| {
            let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
            panic!("expected 'OnCombatLogEvent' in names: {names:?}")
        });
    assert_eq!(sym.kind, SymbolKind::Method);
    // parent == "MyAddon"
    assert!(
        sym.qualified_name.starts_with("MyAddon"),
        "qualified_name should start with 'MyAddon', got: {:?}",
        sym.qualified_name
    );
    assert!(
        sym.summary.contains("Handle combat log event"),
        "docstring should contain 'Handle combat log event', got: {:?}",
        sym.summary
    );
}
///
/// Reference asserts: LANGUAGE_EXTENSIONS[".lua"] == "lua"
/// Rust equivalent: LanguageRegistry resolves "addon.lua" to the lua extractor.
#[test]
fn test_lua_extension_registered() {
    let registry = LanguageRegistry::default();
    let ext = registry.extractor_for_path(Path::new("addon.lua"));
    assert!(ext.is_some(), ".lua extension should be registered");
    assert_eq!(
        ext.unwrap().id().as_str(),
        "lua",
        ".lua should resolve to language 'lua'"
    );
}
