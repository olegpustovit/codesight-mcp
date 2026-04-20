//! Builtin tree-sitter grammar bindings and query packs.
//!
//! Phase C moves grammar ownership out of `codesight-extractors` into the
//! language crate. Each builtin language declares a `&'static GrammarSpec`
//! with the tree-sitter loader, grammar name, and inline `tags.scm` /
//! optional `injections.scm`. Manifests reference these as
//! `grammar: Some(&RUST_GRAMMAR)`.
//!
//! `builtin_query_packs()` exposes the same `(pack_id, tags, injections,
//! metadata)` table the extractor previously owned, so
//! `codesight-extractors::QueryPackCatalog` can delegate without behaviour
//! change.

use std::sync::OnceLock;

use tree_sitter::Language;

use crate::manifest::GrammarSpec;

include!(concat!(env!("OUT_DIR"), "/builtin_query_packs.rs"));

/// Content-addressed stamp over the built-in query pack payload plus the
/// crate version. Used by the indexer to detect that extractor behaviour has
/// changed even when no source file on disk has — tags.scm edits, grammar
/// bumps, and new built-in packs all change the stamp, forcing a rehash of
/// previously-cached files on next incremental ingest.
pub fn builtin_extractor_version_stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP
        .get_or_init(|| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
            hasher.update(b"\0");
            for (pack_id, query, injections, metadata) in BUILTIN_QUERY_PACKS {
                hasher.update(pack_id.as_bytes());
                hasher.update(b"\0");
                hasher.update(query.as_bytes());
                hasher.update(b"\0");
                hasher.update(injections.unwrap_or("").as_bytes());
                hasher.update(b"\0");
                hasher.update(metadata.unwrap_or("").as_bytes());
                hasher.update(b"\0");
            }
            hasher.finalize().to_hex().to_string()
        })
        .as_str()
}

/// Same shape as the generated `BUILTIN_QUERY_PACKS` rows.
pub fn builtin_query_packs() -> &'static [BuiltinQueryPackRow] {
    BUILTIN_QUERY_PACKS
}

/// Resolve a compiled tree-sitter `Language` by the `grammar_name` declared in
/// a `GrammarSpec`. Returns `None` for unknown grammars.
pub fn builtin_grammar(name: &str) -> Option<Language> {
    BUILTIN_GRAMMARS
        .iter()
        .find(|spec| spec.grammar_name == name)
        .map(|spec| (spec.loader)())
}

/// Iterate every registered builtin `GrammarSpec`. Used by Phase C adapters
/// in `codesight-extractors` to preserve `GrammarCatalog::iter` semantics.
pub fn builtin_grammars() -> &'static [&'static GrammarSpec] {
    BUILTIN_GRAMMARS
}

// Tree-sitter crates expose their root `Language` under slightly different
// names (`LANGUAGE`, `LANGUAGE_PHP`, `LANGUAGE_TSX`, `LANGUAGE_XML`, …).
// `scss::language()` uses a fn, not a const. One loader fn per grammar keeps
// the macro call sites uniform.

fn load_bash() -> Language {
    tree_sitter_bash::LANGUAGE.into()
}
fn load_csharp() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}
fn load_cpp() -> Language {
    tree_sitter_cpp::LANGUAGE.into()
}
fn load_css() -> Language {
    tree_sitter_css::LANGUAGE.into()
}
fn load_dart() -> Language {
    tree_sitter_dart::LANGUAGE.into()
}
fn load_elixir() -> Language {
    tree_sitter_elixir::LANGUAGE.into()
}
fn load_ejs() -> Language {
    tree_sitter_embedded_template::LANGUAGE.into()
}
fn load_erlang() -> Language {
    tree_sitter_erlang::LANGUAGE.into()
}
fn load_fortran() -> Language {
    tree_sitter_fortran::LANGUAGE.into()
}
fn load_gleam() -> Language {
    tree_sitter_gleam::LANGUAGE.into()
}
fn load_go() -> Language {
    tree_sitter_go::LANGUAGE.into()
}
fn load_java() -> Language {
    tree_sitter_java::LANGUAGE.into()
}
fn load_javascript() -> Language {
    tree_sitter_javascript::LANGUAGE.into()
}
fn load_json() -> Language {
    tree_sitter_json::LANGUAGE.into()
}
fn load_kotlin() -> Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}
fn load_lua() -> Language {
    tree_sitter_lua::LANGUAGE.into()
}
fn load_luau() -> Language {
    tree_sitter_luau::LANGUAGE.into()
}
fn load_nix() -> Language {
    tree_sitter_nix::LANGUAGE.into()
}
fn load_perl() -> Language {
    tree_sitter_perl::LANGUAGE.into()
}
fn load_php() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}
fn load_python() -> Language {
    tree_sitter_python::LANGUAGE.into()
}
fn load_ruby() -> Language {
    tree_sitter_ruby::LANGUAGE.into()
}
fn load_rust() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}
fn load_scss() -> Language {
    tree_sitter_scss::language()
}
fn load_sql() -> Language {
    tree_sitter_sequel::LANGUAGE.into()
}
fn load_swift() -> Language {
    tree_sitter_swift::LANGUAGE.into()
}
fn load_typescript() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn load_tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
fn load_xml() -> Language {
    tree_sitter_xml::LANGUAGE_XML.into()
}
fn load_yaml() -> Language {
    tree_sitter_yaml::LANGUAGE.into()
}

pub static BASH_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "bash",
    loader: load_bash,
    tags_scm: include_str!("../runtime/queries/bash/tags.scm"),
    injections_scm: None,
};
pub static CSHARP_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "c_sharp",
    loader: load_csharp,
    tags_scm: include_str!("../runtime/queries/csharp/tags.scm"),
    injections_scm: None,
};
pub static CPP_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "cpp",
    loader: load_cpp,
    tags_scm: include_str!("../runtime/queries/cpp/tags.scm"),
    injections_scm: None,
};
pub static CSS_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "css",
    loader: load_css,
    tags_scm: include_str!("../runtime/queries/css/tags.scm"),
    injections_scm: None,
};
pub static DART_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "dart",
    loader: load_dart,
    tags_scm: include_str!("../runtime/queries/dart/tags.scm"),
    injections_scm: None,
};
pub static ELIXIR_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "elixir",
    loader: load_elixir,
    tags_scm: include_str!("../runtime/queries/elixir/tags.scm"),
    injections_scm: None,
};
pub static EJS_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "embedded_template",
    loader: load_ejs,
    tags_scm: include_str!("../runtime/queries/ejs/tags.scm"),
    injections_scm: Some(include_str!("../runtime/queries/ejs/injections.scm")),
};
pub static ERLANG_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "erlang",
    loader: load_erlang,
    tags_scm: include_str!("../runtime/queries/erlang/tags.scm"),
    injections_scm: None,
};
pub static FORTRAN_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "fortran",
    loader: load_fortran,
    tags_scm: include_str!("../runtime/queries/fortran/tags.scm"),
    injections_scm: None,
};
pub static GLEAM_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "gleam",
    loader: load_gleam,
    tags_scm: include_str!("../runtime/queries/gleam/tags.scm"),
    injections_scm: None,
};
pub static GO_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "go",
    loader: load_go,
    tags_scm: include_str!("../runtime/queries/go/tags.scm"),
    injections_scm: None,
};
pub static JAVA_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "java",
    loader: load_java,
    tags_scm: include_str!("../runtime/queries/java/tags.scm"),
    injections_scm: None,
};
pub static JAVASCRIPT_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "javascript",
    loader: load_javascript,
    tags_scm: include_str!("../runtime/queries/javascript/tags.scm"),
    injections_scm: Some(include_str!(
        "../runtime/queries/javascript/injections.scm"
    )),
};
pub static JSON_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "json",
    loader: load_json,
    tags_scm: include_str!("../runtime/queries/json/tags.scm"),
    injections_scm: None,
};
pub static KOTLIN_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "kotlin",
    loader: load_kotlin,
    tags_scm: include_str!("../runtime/queries/kotlin/tags.scm"),
    injections_scm: None,
};
pub static LUA_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "lua",
    loader: load_lua,
    tags_scm: include_str!("../runtime/queries/lua/tags.scm"),
    injections_scm: None,
};
pub static LUAU_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "luau",
    loader: load_luau,
    tags_scm: include_str!("../runtime/queries/luau/tags.scm"),
    injections_scm: None,
};
pub static NIX_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "nix",
    loader: load_nix,
    tags_scm: include_str!("../runtime/queries/nix/tags.scm"),
    injections_scm: None,
};
pub static PERL_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "perl",
    loader: load_perl,
    tags_scm: include_str!("../runtime/queries/perl/tags.scm"),
    injections_scm: None,
};
pub static PHP_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "php",
    loader: load_php,
    tags_scm: include_str!("../runtime/queries/php/tags.scm"),
    injections_scm: None,
};
pub static PYTHON_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "python",
    loader: load_python,
    tags_scm: include_str!("../runtime/queries/python/tags.scm"),
    injections_scm: None,
};
pub static RUBY_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "ruby",
    loader: load_ruby,
    tags_scm: include_str!("../runtime/queries/ruby/tags.scm"),
    injections_scm: None,
};
pub static RUST_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "rust",
    loader: load_rust,
    tags_scm: include_str!("../runtime/queries/rust/tags.scm"),
    injections_scm: Some(include_str!("../runtime/queries/rust/injections.scm")),
};
pub static SCSS_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "scss",
    loader: load_scss,
    tags_scm: include_str!("../runtime/queries/scss/tags.scm"),
    injections_scm: None,
};
pub static SQL_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "sql",
    loader: load_sql,
    tags_scm: include_str!("../runtime/queries/sql/tags.scm"),
    injections_scm: None,
};
pub static SWIFT_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "swift",
    loader: load_swift,
    tags_scm: include_str!("../runtime/queries/swift/tags.scm"),
    injections_scm: None,
};
pub static TYPESCRIPT_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "typescript",
    loader: load_typescript,
    tags_scm: include_str!("../runtime/queries/typescript/tags.scm"),
    injections_scm: Some(include_str!(
        "../runtime/queries/typescript/injections.scm"
    )),
};
pub static TSX_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "tsx",
    loader: load_tsx,
    tags_scm: include_str!("../runtime/queries/tsx/tags.scm"),
    injections_scm: None,
};
pub static XML_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "xml",
    loader: load_xml,
    tags_scm: include_str!("../runtime/queries/xml/tags.scm"),
    injections_scm: None,
};
pub static YAML_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "yaml",
    loader: load_yaml,
    tags_scm: include_str!("../runtime/queries/yaml/tags.scm"),
    injections_scm: None,
};

/// Every builtin grammar spec. Order is not significant; callers look up by
/// `grammar_name`.
static BUILTIN_GRAMMARS: &[&GrammarSpec] = &[
    &BASH_GRAMMAR,
    &CSHARP_GRAMMAR,
    &CPP_GRAMMAR,
    &CSS_GRAMMAR,
    &DART_GRAMMAR,
    &ELIXIR_GRAMMAR,
    &EJS_GRAMMAR,
    &ERLANG_GRAMMAR,
    &FORTRAN_GRAMMAR,
    &GLEAM_GRAMMAR,
    &GO_GRAMMAR,
    &JAVA_GRAMMAR,
    &JAVASCRIPT_GRAMMAR,
    &JSON_GRAMMAR,
    &KOTLIN_GRAMMAR,
    &LUA_GRAMMAR,
    &LUAU_GRAMMAR,
    &NIX_GRAMMAR,
    &PERL_GRAMMAR,
    &PHP_GRAMMAR,
    &PYTHON_GRAMMAR,
    &RUBY_GRAMMAR,
    &RUST_GRAMMAR,
    &SCSS_GRAMMAR,
    &SQL_GRAMMAR,
    &SWIFT_GRAMMAR,
    &TYPESCRIPT_GRAMMAR,
    &TSX_GRAMMAR,
    &XML_GRAMMAR,
    &YAML_GRAMMAR,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_grammar_has_nonempty_tags_and_loads() {
        for spec in BUILTIN_GRAMMARS {
            assert!(
                !spec.tags_scm.is_empty(),
                "{} tags.scm is empty",
                spec.grammar_name
            );
            let _ = (spec.loader)();
        }
    }

    #[test]
    fn builtin_grammar_lookup_matches_grammar_name() {
        assert!(builtin_grammar("rust").is_some());
        assert!(builtin_grammar("c_sharp").is_some());
        assert!(builtin_grammar("tsx").is_some());
        assert!(builtin_grammar("nope").is_none());
    }

    #[test]
    fn query_pack_table_is_populated() {
        let packs = builtin_query_packs();
        assert!(packs.len() >= 25);
        let rust = packs.iter().find(|(id, _, _, _)| *id == "rust").unwrap();
        assert!(!rust.1.is_empty());
    }

    #[test]
    fn extractor_version_stamp_is_stable_hex() {
        let a = builtin_extractor_version_stamp();
        let b = builtin_extractor_version_stamp();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
