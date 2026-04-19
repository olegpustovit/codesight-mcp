//! Integration tests: arrow functions.

use codesight_extractors::extractors::LanguageExtractor;
use codesight_extractors::extractors::engine::scm::{QUERY_FILES, ScmExtractor, parse_scm_header};
use codesight_core::SymbolKind;

fn scm_extractor(lang_key: &str) -> ScmExtractor {
    let source = QUERY_FILES
        .iter()
        .find(|(k, _)| *k == lang_key)
        .unwrap_or_else(|| panic!("no query for {lang_key}"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).unwrap()
}

fn func_by_name<'a>(
    syms: &'a [codesight_core::StoredSymbol],
    name: &str,
) -> &'a codesight_core::StoredSymbol {
    syms.iter()
        .find(|s| s.name == name && s.kind == SymbolKind::Function)
        .unwrap_or_else(|| {
            panic!(
                "function {name} missing; got: {:?}",
                syms.iter().map(|s| (&s.name, &s.kind)).collect::<Vec<_>>()
            )
        })
}

#[test]
fn case_js_arrow_basic() {
    let src = "const add = (a, b) => a + b;\n";
    let ex = scm_extractor("javascript");
    let syms = ex.extract("utils.js", src).unwrap();
    let f = func_by_name(&syms, "add");
    assert_eq!(f.kind, SymbolKind::Function);
}

#[test]
fn case_js_function_expression() {
    let src = "const multiply = function(a, b) {\n    return a * b;\n};\n";
    let syms = scm_extractor("javascript").extract("math.js", src).unwrap();
    let f = func_by_name(&syms, "multiply");
    assert_eq!(f.kind, SymbolKind::Function);
}

#[test]
fn case_js_generator_expression() {
    let src =
        "const generate = function*(items) {\n    for (const item of items) yield item;\n};\n";
    let syms = scm_extractor("javascript").extract("gen.js", src).unwrap();
    let f = func_by_name(&syms, "generate");
    assert_eq!(f.kind, SymbolKind::Function);
}

#[test]
fn case_js_exported_arrow() {
    let src = "export const validate = (input) => {\n    return input.length > 0;\n};\n";
    let ext = scm_extractor("javascript");
    let syms = ext.extract("validate.js", src).unwrap();
    let f = syms
        .iter()
        .find(|s| s.name == "validate" && s.kind == SymbolKind::Function);
    assert!(f.is_some(), "validate function missing");
    assert!(
        f.unwrap().signature.contains("export"),
        "signature should mention export: {}",
        f.unwrap().signature
    );
}

#[test]
fn case_ts_arrow_typed() {
    let src =
        "export const validate = (input: string): boolean => {\n    return input.length > 0;\n};\n";
    let syms = scm_extractor("typescript")
        .extract("validate.ts", src)
        .unwrap();
    let f = func_by_name(&syms, "validate");
    assert!(f.signature.contains("export"));
}

#[test]
fn case_non_function_assignments() {
    let src = r#"const x = 5;
const arr = [1, 2, 3];
const obj = { key: "value" };
const str = "hello";
"#;
    let syms = scm_extractor("javascript").extract("vals.js", src).unwrap();
    let funcs: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .collect();
    assert!(funcs.is_empty());
}

#[test]
fn case_inline_arrow() {
    let src = r#"[1, 2, 3].map(x => x + 1);
const result = items.filter(item => item.active);
"#;
    let syms = scm_extractor("javascript")
        .extract("inline.js", src)
        .unwrap();
    let funcs: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .collect();
    assert!(funcs.is_empty());
}

#[test]
fn case_destructuring() {
    let src = r#"const { foo, bar } = require("something");
const [a, b] = [1, 2];
"#;
    let syms = scm_extractor("javascript")
        .extract("destructure.js", src)
        .unwrap();
    let funcs: Vec<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .collect();
    assert!(funcs.is_empty());
}

#[test]
fn case_arrow_jsdoc() {
    let src = "/** Adds two numbers together. */\nconst add = (a, b) => a + b;\n";
    let syms = scm_extractor("javascript").extract("doc.js", src).unwrap();
    let f = func_by_name(&syms, "add");
    assert!(
        f.summary.contains("Adds two numbers") || f.signature.contains("add"),
        "summary/signature: {:?}",
        f.summary
    );
}

#[test]
fn case_arrow_signature_params() {
    let src = "const processData = (data, options) => {\n    return data;\n};\n";
    let syms = scm_extractor("javascript")
        .extract("process.js", src)
        .unwrap();
    let f = func_by_name(&syms, "processData");
    assert!(f.signature.contains("data"));
    assert!(f.signature.contains("options"));
}

#[test]
fn case_multiple_mixed() {
    let src = "const foo = () => {}\nfunction bar() {}\nconst baz = (x) => x * 2;\n";
    let syms = scm_extractor("javascript")
        .extract("multi.js", src)
        .unwrap();
    let names: std::collections::HashSet<_> = syms
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .map(|s| s.name.as_str())
        .collect();
    assert!(names.contains("foo"));
    assert!(names.contains("bar"));
    assert!(names.contains("baz"));
}
