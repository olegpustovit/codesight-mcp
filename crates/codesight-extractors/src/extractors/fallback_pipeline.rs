//! Structural-first symbol extraction pipeline (crate-internal convention).
//!
//! ## Order
//!
//! 1. **Structural pass** — tree-sitter parse + AST walk and/or `.scm` queries.
//! 2. **Named fallbacks** — regex, line scans, or other heuristics **only** for
//!    documented grammar gaps (jinja-in-SQL, fragment parsing, etc.).
//! 3. **Merge** — [`sort_symbols_by_span`] then [`dedup_symbols_by_start_byte`].
//!
//! ## Span collisions
//!
//! Push structural symbols **before** fallbacks. When both cover the same
//! `start_byte`, [`dedup_symbols_by_start_byte`] keeps the **first** occurrence,
//! so structural hits win over regex duplicates.

use crate::core::pipeline::StoredSymbol;

/// Sort symbols by start byte, then end byte (stable ordering for diagnostics).
pub fn sort_symbols_by_span(symbols: &mut [StoredSymbol]) {
    symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
}

/// Drop later symbols that share the same `start_byte` as an earlier one.
///
/// Call after a structural pass appended fallback symbols, with structural
/// symbols ordered first (or sort so structural rows sort before fallback for
/// the same `start_byte` — typically structural spans differ; SQL aligns both
/// on the same `start_byte` only when they represent the same construct).
pub fn dedup_symbols_by_start_byte(symbols: &mut Vec<StoredSymbol>) {
    symbols.dedup_by_key(|s| s.start_byte);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::symbols::{ExtractionTier, SymbolKind};

    fn dummy_sym(start: usize, name: &str) -> StoredSymbol {
        StoredSymbol {
            id: format!("f::{name}"),
            extraction_tier: ExtractionTier::ManualOverride,
            language: "x".into(),
            file_path: "f".into(),
            qualified_name: name.into(),
            name: name.into(),
            kind: SymbolKind::Function,
            signature: String::new(),
            summary: String::new(),
            start_line: 1,
            end_line: 1,
            start_byte: start,
            end_byte: start + 1,
            source_hash: String::new(),
            source_text: String::new(),
            file_size_bytes: 0,
        }
    }

    #[test]
    fn dedup_keeps_first_start_byte() {
        let mut v = vec![dummy_sym(1, "a"), dummy_sym(1, "b"), dummy_sym(2, "c")];
        dedup_symbols_by_start_byte(&mut v);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "a");
        assert_eq!(v[1].name, "c");
    }
}
