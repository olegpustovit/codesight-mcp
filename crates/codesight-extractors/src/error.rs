//! Tree-sitter and extractor-domain failures. Convert to `anyhow::Error` at registry boundaries via `?`.

use thiserror::Error;

/// Result alias for extractors and parse helpers.
pub type ExtractResult<T> = Result<T, ExtractError>;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("failed to load tree-sitter grammar")]
    GrammarLoad,
    #[error("tree-sitter returned no parse tree")]
    MissingParseTree,
    #[error("tree-sitter node was not valid utf-8")]
    InvalidUtf8,
    #[error("tree-sitter byte range was out of bounds")]
    ByteRangeOutOfBounds,
}
