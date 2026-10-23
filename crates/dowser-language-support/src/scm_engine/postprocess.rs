//! Postprocess phase for query-pack extraction.
//!
//! Generic dispatch: after the SCM engine finalizes a symbol list, we look
//! up the [`Language`] for the file's language name and delegate to its
//! `postprocess_symbols` override. C++'s plain-C header downgrade, TSX/JSX
//! content probes, and any future per-language content-aware fixups live
//! on the trait, not here.

use dowser_core::StoredSymbol;

use super::ScmExtractor;

pub(super) fn run_postprocesses(
    extractor: &ScmExtractor,
    file_path: &str,
    source: &str,
    symbols: &mut [StoredSymbol],
) {
    let Some(lang) = crate::builtin_registry().for_language_name(extractor.language_name) else {
        return;
    };
    lang.postprocess_symbols(symbols, file_path, source);
}
