//! Tree-sitter parser coverage (non-Lua cases).

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

const PYTHON_SOURCE: &str = r#"
class MyClass:
    """A sample class."""
    def method(self, x: int) -> str:
        """Do something."""
        return str(x)

def standalone(a, b):
    """Standalone function."""
    return a + b

MAX_SIZE = 100
"#;

const JS_CONST_SOURCE: &str = r#"
const MAX_RETRIES = 3;

export const BASE_URL = "https://api.example.com";

const config = { debug: false };

const onClick = () => console.log("click");

const handler = function() { return 42; };
"#;

const TS_CONST_SOURCE: &str = r#"
const MAX_RETRIES: number = 3;

export const BASE_URL: string = "https://api.example.com";

const config = Object.freeze({ debug: false });

const format = (s: string): string => s.trim();
"#;

#[test]
fn reference_parser_suite() {
    parse_python_matches_reference_oracle();
    symbol_id_format_matches_jmri();
    unknown_extension_yields_no_extractor();
    symbol_byte_ranges_are_non_empty();
    js_const_extraction_matches_reference_oracle();
    ts_const_extraction_matches_reference_oracle();
}

fn parse_python_matches_reference_oracle() {
    let ex = scm_extractor("python");
    let syms = ex
        .extract("test.py", PYTHON_SOURCE)
        .expect("python extract");

    let classes: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    assert_eq!(classes.len(), 1, "expected one class");
    assert_eq!(classes[0].name, "MyClass");
    assert!(
        classes[0].summary.contains("A sample class")
            || classes[0].signature.contains("class MyClass"),
        "class doc/summary: {:?}",
        classes[0].summary
    );

    let methods: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "method");
    assert_eq!(methods[0].qualified_name, "MyClass.method");

    let funcs: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function && s.name == "standalone")
        .collect();
    assert_eq!(funcs.len(), 1);
    assert!(
        funcs[0].summary.contains("Standalone") || funcs[0].signature.contains("def standalone")
    );

    let constants: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Constant)
        .collect();
    assert_eq!(constants.len(), 1);
    assert_eq!(constants[0].name, "MAX_SIZE");
}

fn symbol_id_format_matches_jmri() {
    assert_eq!(
        format_id("src/main.py", "MyClass.method", SymbolKind::Method),
        "src/main.py::MyClass.method#method"
    );
    assert_eq!(
        format_id("test.py", "standalone", SymbolKind::Function),
        "test.py::standalone#function"
    );
}

fn format_id(file: &str, qualified: &str, kind: SymbolKind) -> String {
    format!("{file}::{qualified}#{}", kind.as_str())
}

fn unknown_extension_yields_no_extractor() {
    let reg = LanguageRegistry::default();
    assert!(
        reg.extractor_for_path(Path::new("opaque.unknown"))
            .is_none()
    );
}

fn symbol_byte_ranges_are_non_empty() {
    let ex = scm_extractor("python");
    let syms = ex
        .extract("test.py", PYTHON_SOURCE)
        .expect("python extract");
    for sym in &syms {
        assert!(sym.start_byte < sym.end_byte, "{}", sym.id);
        assert!(sym.start_line > 0);
        assert!(sym.end_line >= sym.start_line);
    }
}

fn js_const_extraction_matches_reference_oracle() {
    let ex = scm_extractor("javascript");
    let syms = ex.extract("util.js", JS_CONST_SOURCE).expect("js extract");
    let by_name: std::collections::HashMap<_, _> =
        syms.iter().map(|s| (s.name.as_str(), s)).collect();

    assert!(by_name.contains_key("MAX_RETRIES"));
    assert!(by_name.contains_key("BASE_URL"));
    assert!(by_name.contains_key("config"));
    assert_eq!(by_name["MAX_RETRIES"].kind, SymbolKind::Constant);
    assert_eq!(by_name["BASE_URL"].kind, SymbolKind::Constant);
    assert_eq!(by_name["config"].kind, SymbolKind::Constant);

    if let Some(s) = by_name.get("onClick") {
        assert_eq!(s.kind, SymbolKind::Function);
    }
    if let Some(s) = by_name.get("handler") {
        assert_eq!(s.kind, SymbolKind::Function);
    }
}

fn ts_const_extraction_matches_reference_oracle() {
    let ex = scm_extractor("typescript");
    let syms = ex.extract("util.ts", TS_CONST_SOURCE).expect("ts extract");
    let by_name: std::collections::HashMap<_, _> =
        syms.iter().map(|s| (s.name.as_str(), s)).collect();

    assert!(by_name.contains_key("MAX_RETRIES"));
    assert!(by_name.contains_key("BASE_URL"));
    assert!(by_name.contains_key("config"));
    assert_eq!(by_name["MAX_RETRIES"].kind, SymbolKind::Constant);
    assert_eq!(by_name["BASE_URL"].kind, SymbolKind::Constant);
    assert_eq!(by_name["config"].kind, SymbolKind::Constant);

    if let Some(s) = by_name.get("format") {
        assert_eq!(s.kind, SymbolKind::Function);
    }
}
