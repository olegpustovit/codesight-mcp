//! Declarative extractor engine driven by `.scm` query files.
//!
//! Each language is defined by a single `runtime/queries/{lang}/symbols.scm` file
//! containing tree-sitter query patterns. The engine compiles the query, runs it
//! via `QueryCursor`, and builds `StoredSymbol` records from captures using
//! containment-based scope inference. **Symbol capture** is query-driven; a tiny
//! named post-pass ([`cpp_header_language_fallback`]) adjusts reported language
//! where the grammar cannot encode the distinction (C vs C++ headers).
//!
//! See `scm.rs` for the implementation and `runtime/queries/` for language definitions.

pub(crate) mod cpp_header_language_fallback;
pub mod scm;

/// Map grammar name (from .scm header) to a compiled-in tree-sitter Language.
///
/// This is the single place where tree-sitter grammar crates are referenced.
/// To add a new grammar: add the crate to `Cargo.toml` and a match arm here.
pub fn builtin_grammar(name: &str) -> Option<tree_sitter::Language> {
    Some(match name {
        "bash" => tree_sitter_bash::LANGUAGE.into(),
        "c_sharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "css" => tree_sitter_css::LANGUAGE.into(),
        "dart" => tree_sitter_dart::LANGUAGE.into(),
        "elixir" => tree_sitter_elixir::LANGUAGE.into(),
        "erlang" => tree_sitter_erlang::LANGUAGE.into(),
        "fortran" => tree_sitter_fortran::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "lua" => tree_sitter_lua::LANGUAGE.into(),
        "luau" => tree_sitter_luau::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "scss" => tree_sitter_scss::language().into(),
        "sql" => tree_sitter_sequel::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "yaml" => tree_sitter_yaml::LANGUAGE.into(),
        "gleam" => tree_sitter_gleam::LANGUAGE.into(),
        "nix" => tree_sitter_nix::LANGUAGE.into(),
        "perl" => tree_sitter_perl::LANGUAGE.into(),
        "xml" => tree_sitter_xml::LANGUAGE_XML.into(),
        _ => return None,
    })
}
