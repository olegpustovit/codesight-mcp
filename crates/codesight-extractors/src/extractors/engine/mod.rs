//! Declarative extractor engine driven by `.scm` query files.
//!
//! Built-in packs live at
//! `codesight-language-support/runtime/queries/<pack_id>/tags.scm` (with
//! optional `injections.scm` + `pack.toml`). The engine reads packs through
//! [`catalogs::QueryPackCatalog`] and grammars through
//! [`codesight_language_support::builtin_grammar`]. The engine compiles each
//! query, runs it via `QueryCursor`, and builds `StoredSymbol` records
//! using containment-based scope inference. **Symbol capture** is
//! query-driven; a tiny named post-pass
//! ([`cpp_header_language_fallback`]) adjusts reported language where the
//! grammar cannot encode the distinction (C vs C++ headers).

pub mod catalogs;
pub(crate) mod cpp_header_language_fallback;
pub mod generic_ast;
pub mod scm;

/// Map grammar name (from .scm header) to a compiled-in tree-sitter Language.
///
/// Grammar ownership lives in `codesight-language-support::grammars`.
pub fn builtin_grammar(name: &str) -> Option<tree_sitter::Language> {
    codesight_language_support::builtin_grammar(name)
}
