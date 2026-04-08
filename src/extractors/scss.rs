//! Residual extractor facade.
//!
//! Category: `multi_dialect`.
//! Exit criteria: delete these facades once the stylesheet preprocessor family is covered by
//! curated packs or grammar support without Rust dialect-specific traversal.

pub use crate::extractors::residual::multi_dialect::scss::{
    LessExtractor, SassExtractor, ScssExtractor, StylExtractor,
};
