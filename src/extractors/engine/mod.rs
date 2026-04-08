//! Declarative extractor engine driven by `.scm` query files.
//!
//! Each built-in pack is a `runtime/queries/<pack_id>/tags.scm` file with tree-sitter query
//! patterns. Optional `injections.scm` and `pack.toml` files live beside it. **`build.rs`
//! discovers packs** and embeds them; the engine compiles each query, runs it via
//! `QueryCursor`, and builds `StoredSymbol` records using containment-based scope inference.
//! **Symbol capture** is query-driven; a tiny named post-pass
//! ([`cpp_header_language_fallback`]) adjusts reported language where the grammar cannot encode
//! the distinction (C vs C++ headers).
//!
//! See `scm/` for the implementation phases and `runtime/queries/` for pack sources.

pub mod catalogs;
pub(crate) mod cpp_header_language_fallback;
pub mod generic_ast;
pub mod scm;

/// Map grammar name (from .scm header) to a compiled-in tree-sitter Language.
///
/// Grammar ownership lives in [`catalogs::GrammarCatalog`].
pub fn builtin_grammar(name: &str) -> Option<tree_sitter::Language> {
    catalogs::GrammarCatalog::builtin()
        .find_by_grammar(name)
        .map(|definition| definition.language())
}
