//! Symbol extraction axis (LANGUAGES.md §4.1).
//!
//! Phase A keeps the trait surface deliberately small — `supports_extraction`
//! gates whether downstream crates should call this language through the
//! registry at all. Phase D grows this trait to carry the full extraction
//! contract (primary strategy, docstrings, `SymbolKind` mapping,
//! `ExtractionTier`, wrapper delegation, source normalization).

pub trait SymbolExtraction: Send + Sync {
    /// Whether this language produces extracted symbols. Languages without a
    /// grammar or a manual extractor return `false`.
    fn supports_extraction(&self) -> bool {
        false
    }
}

/// Default: no extraction. Used by every language until it opts in.
pub struct NoopExtraction;

impl SymbolExtraction for NoopExtraction {}
