//! Integration tests: luau language.

use std::path::Path;

use codesight_core::SymbolKind;
use codesight_language_support::LanguageRegistry;
use codesight_language_support::scm_engine::{ScmExtractor, parse_scm_header, query_files_slice};

const LUAU_SERVICE: &str = "\
-- RaidService handles raid mechanics between players
local RaidService = {}
RaidService.__index = RaidService

export type RaidConfig = {
    duration: number,
    maxPlayers: number,
    rewards: {string},
}

type State = \"idle\" | \"active\" | \"complete\"

-- Create a new RaidService instance
function RaidService.new(config: RaidConfig): RaidService
    local self = setmetatable({}, RaidService)
    self.config = config
    return self
end

-- Start the raid
function RaidService:Start()
    self.active = true
end

function RaidService:GetRewards(player: Player, multiplier: number): {string}
    return {}
end

local function helperFunction(x: number, y: number): number
    return x + y
end
";

fn luau_extractor() -> ScmExtractor {
    let source = query_files_slice()
        .iter()
        .find(|(key, _)| *key == "luau")
        .unwrap_or_else(|| panic!("no query pack for luau"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).expect("luau extractor")
}

fn luau_symbols() -> Vec<codesight_core::StoredSymbol> {
    luau_extractor()
        .extract("RaidService.luau", LUAU_SERVICE)
        .expect("luau extraction failed")
}
/// Smoke test: the Luau grammar must load without panicking.
#[test]
fn test_luau_parser_available() {
    let result = luau_extractor().extract("smoke.luau", "local x = 1\n");
    assert!(
        result.is_ok(),
        "luau grammar must load: {}",
        result.unwrap_err()
    );
}
/// Reference asserts: len(symbols) >= 5
#[test]
fn test_luau_symbols_extracted() {
    let symbols = luau_symbols();
    assert!(
        symbols.len() >= 5,
        "expected at least 5 symbols, got {}: {:?}",
        symbols.len(),
        symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
/// Reference asserts:
///   - name == "new", kind == "method"
///   - qualified_name == "RaidService.new"
///   - parent == "RaidService"
#[test]
fn test_luau_module_method() {
    let symbols = luau_symbols();
    let new_fn = symbols
        .iter()
        .find(|s| s.name == "new")
        .expect("expected symbol named 'new'");
    assert_eq!(new_fn.kind, SymbolKind::Method);
    assert_eq!(new_fn.qualified_name, "RaidService.new");
    // parent == "RaidService" encoded in qualified_name prefix
    assert!(new_fn.qualified_name.starts_with("RaidService."));
}
/// Reference asserts:
///   - name == "Start", kind == "method"
///   - qualified_name == "RaidService:Start"
///   - parent == "RaidService"
///
/// NOTE: The Rust extractor's qualify() joins with '.', producing
/// "RaidService.Start" instead of "RaidService:Start".
#[test]
fn test_luau_oop_method_qualified_name_uses_colon() {
    let symbols = luau_symbols();
    let start = symbols
        .iter()
        .find(|s| s.name == "Start")
        .expect("expected symbol named 'Start'");
    assert_eq!(start.kind, SymbolKind::Method);
    assert_eq!(start.qualified_name, "RaidService:Start");
}

/// Non-ignored portion of test_luau_oop_method: kind and parent.
#[test]
fn test_luau_oop_method_kind_and_parent() {
    let symbols = luau_symbols();
    let start = symbols
        .iter()
        .find(|s| s.name == "Start")
        .expect("expected symbol named 'Start'");
    assert_eq!(start.kind, SymbolKind::Method);
    assert!(
        start.qualified_name.starts_with("RaidService"),
        "qualified_name should start with 'RaidService', got: {:?}",
        start.qualified_name
    );
}
/// Reference asserts:
///   - name == "helperFunction", kind == "function"
///   - parent is None
///   - "local function" in signature
#[test]
fn test_luau_local_function() {
    let symbols = luau_symbols();
    let helper = symbols
        .iter()
        .find(|s| s.name == "helperFunction")
        .expect("expected symbol named 'helperFunction'");
    assert_eq!(helper.kind, SymbolKind::Function);
    // parent is None => no dot in qualified_name
    assert!(
        !helper.qualified_name.contains('.'),
        "helperFunction should have no parent scope, got qualified_name={:?}",
        helper.qualified_name
    );
    assert!(
        helper.signature.contains("local function"),
        "signature should contain 'local function', got: {:?}",
        helper.signature
    );
}
/// Reference asserts:
///   - GetRewards signature contains "player: Player"
///   - GetRewards signature contains "multiplier: number"
#[test]
fn test_luau_typed_params_in_signature() {
    let symbols = luau_symbols();
    let rewards = symbols
        .iter()
        .find(|s| s.name == "GetRewards")
        .expect("expected symbol named 'GetRewards'");
    assert!(
        rewards.signature.contains("player: Player"),
        "signature should contain 'player: Player', got: {:?}",
        rewards.signature
    );
    assert!(
        rewards.signature.contains("multiplier: number"),
        "signature should contain 'multiplier: number', got: {:?}",
        rewards.signature
    );
}
/// Reference asserts:
///   - helperFunction signature contains ": number"
#[test]
fn test_luau_return_type_in_signature() {
    let symbols = luau_symbols();
    let helper = symbols
        .iter()
        .find(|s| s.name == "helperFunction")
        .expect("expected symbol named 'helperFunction'");
    assert!(
        helper.signature.contains(": number"),
        "signature should contain ': number' (return type), got: {:?}",
        helper.signature
    );
}
/// Reference asserts:
///   - RaidConfig found, kind == "type"
///   - "export type" in signature
#[test]
fn test_luau_export_type() {
    let symbols = luau_symbols();
    let raid_config = symbols
        .iter()
        .find(|s| s.name == "RaidConfig")
        .expect("expected symbol named 'RaidConfig'");
    assert_eq!(raid_config.kind, SymbolKind::Type);
    assert!(
        raid_config.signature.contains("export type"),
        "signature should contain 'export type', got: {:?}",
        raid_config.signature
    );
}
/// Reference asserts:
///   - State found, kind == "type"
#[test]
fn test_luau_local_type() {
    let symbols = luau_symbols();
    let state = symbols
        .iter()
        .find(|s| s.name == "State")
        .expect("expected symbol named 'State'");
    assert_eq!(state.kind, SymbolKind::Type);
}
/// Reference asserts:
///   - Start's docstring contains "Start the raid"
#[test]
fn test_luau_docstring() {
    let symbols = luau_symbols();
    let start = symbols
        .iter()
        .find(|s| s.name == "Start")
        .expect("expected symbol named 'Start'");
    assert!(
        start.summary.contains("Start the raid"),
        "docstring/summary should contain 'Start the raid', got: {:?}",
        start.summary
    );
}
/// Reference asserts: all symbols have language == "luau"
#[test]
fn test_luau_language_field() {
    let symbols = luau_symbols();
    assert!(!symbols.is_empty(), "should have at least one symbol");
    for sym in &symbols {
        assert_eq!(
            sym.language.as_str(),
            "luau",
            "symbol {:?} has language {:?}, expected 'luau'",
            sym.name,
            sym.language
        );
    }
}
/// Reference asserts:
///   - Start's signature == "function RaidService:Start()"
///
/// NOTE: The Rust extractor produces "function RaidService:Start()" directly
/// from the first line of the source node. If the tree-sitter node text differs,
/// this test captures the reference expectation.
#[test]
fn test_luau_no_return_type_without_annotation() {
    let symbols = luau_symbols();
    let start = symbols
        .iter()
        .find(|s| s.name == "Start")
        .expect("expected symbol named 'Start'");
    assert_eq!(
        start.signature, "function RaidService:Start()",
        "signature should be exactly 'function RaidService:Start()'"
    );
}
/// Reference asserts: LANGUAGE_EXTENSIONS[".luau"] == "luau"
#[test]
fn test_luau_extension_registered() {
    let registry = LanguageRegistry::default();
    let ext = registry.extractor_for_path(Path::new("module.luau"));
    assert!(ext.is_some(), ".luau extension should be registered");
    assert_eq!(
        ext.unwrap().id().as_str(),
        "luau",
        ".luau should resolve to language 'luau'"
    );
}
/// Reference asserts: "luau" in LANGUAGE_REGISTRY
/// Rust equivalent: LanguageRegistry default includes a luau extractor.
#[test]
fn test_luau_language_in_registry() {
    let registry = LanguageRegistry::default();
    // Verify that any .luau file resolves, confirming luau is in the registry.
    let ext = registry.extractor_for_path(Path::new("test.luau"));
    assert!(ext.is_some(), "luau should be in the extractor registry");
    assert_eq!(ext.unwrap().id().as_str(), "luau");
}
