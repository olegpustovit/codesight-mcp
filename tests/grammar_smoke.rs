//! Smoke tests for all tree-sitter grammars to detect ABI/runtime crashes.
//! Run with: cargo test -p codesight-mcp --test grammar_smoke

fn try_grammar(name: &str, lang: tree_sitter::Language, src: &str) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&lang)
        .unwrap_or_else(|_| panic!("{name}: set_language failed"));
    let tree = parser
        .parse(src, None)
        .unwrap_or_else(|| panic!("{name}: parse returned None"));
    let root = tree.root_node();
    assert!(!root.kind().is_empty(), "{name}: root node kind is empty");
    eprintln!(
        "{name}: OK — root={}, children={}",
        root.kind(),
        root.child_count()
    );
}

#[test]
fn smoke_bash() {
    try_grammar(
        "bash",
        tree_sitter_bash::LANGUAGE.into(),
        "greet() { echo hi; }",
    );
}

#[test]
fn smoke_css() {
    try_grammar(
        "css",
        tree_sitter_css::LANGUAGE.into(),
        "body { margin: 0; }",
    );
}

#[test]
fn smoke_json() {
    try_grammar("json", tree_sitter_json::LANGUAGE.into(), r#"{"a":1}"#);
}

#[test]
fn smoke_lua() {
    try_grammar("lua", tree_sitter_lua::LANGUAGE.into(), "function f() end");
}

#[test]
fn smoke_luau() {
    try_grammar(
        "luau",
        tree_sitter_luau::LANGUAGE.into(),
        "function f() end",
    );
}

#[test]
fn smoke_php() {
    try_grammar(
        "php",
        tree_sitter_php::LANGUAGE_PHP.into(),
        "<?php function f() {}",
    );
}

#[test]
fn smoke_scss() {
    try_grammar("scss", tree_sitter_scss::language(), "$x: 1;");
}

#[test]
fn smoke_sql() {
    try_grammar(
        "sql",
        tree_sitter_sequel::LANGUAGE.into(),
        "CREATE TABLE t (id INT);",
    );
}

#[test]
fn smoke_swift() {
    try_grammar("swift", tree_sitter_swift::LANGUAGE.into(), "func f() {}");
}

#[test]
fn smoke_yaml() {
    try_grammar("yaml", tree_sitter_yaml::LANGUAGE.into(), "key: value");
}
