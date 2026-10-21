//! Builtin tree-sitter grammar + query-pack lookup helpers.
//!
//! Each `GrammarSpec` lives inside its language's module
//! (`src/languages/<lang>/mod.rs`); this module just aggregates them for
//! name-keyed lookup and delivers the generated `BUILTIN_QUERY_PACKS` table.

use std::sync::OnceLock;

use tree_sitter::Language;

use crate::builtin_registry;
use crate::manifest::GrammarSpec;

include!(concat!(env!("OUT_DIR"), "/builtin_query_packs.rs"));

/// Content-addressed stamp over the built-in query pack payload plus the
/// crate version.
pub fn builtin_extractor_version_stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP
        .get_or_init(|| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"import-resolution-quality-v1\0");
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

/// Resolve a compiled tree-sitter `Language` by grammar name.
pub fn builtin_grammar(name: &str) -> Option<Language> {
    builtin_grammar_spec(name).map(|spec| (spec.loader)())
}

/// Resolve the static `GrammarSpec` for a builtin grammar name. Used when
/// workspace-level query packs need to reuse a builtin loader.
pub fn builtin_grammar_spec(name: &str) -> Option<&'static GrammarSpec> {
    builtin_registry()
        .all()
        .filter_map(|lang| lang.grammar_spec())
        .find(|spec| spec.grammar_name == name)
}

/// Iterate every registered builtin `GrammarSpec`.
pub fn builtin_grammars() -> Vec<&'static GrammarSpec> {
    builtin_registry()
        .all()
        .filter_map(|lang| lang.grammar_spec())
        .collect()
}
