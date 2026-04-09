//! Residual extractor facade.
//!
//! Category: `container_boundary`.
//! Exit criteria: delete this facade once EJS scriptlets and partial includes can flow through a
//! shared embedded-template container path without EJS-specific Rust scanning.

pub use crate::extractors::residual::container_boundary::ejs::EjsExtractor;
