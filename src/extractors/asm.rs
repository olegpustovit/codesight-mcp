//! Residual extractor facade.
//!
//! Category: `multi_dialect`.
//! Exit criteria: delete this facade once curated packs or grammar coverage can model GAS, NASM,
//! ca65, and WLA-DX without Rust dialect branching.
//! The internal `AsmDialectStrategy` remains isolated under `extractors::residual`.

pub use crate::extractors::residual::multi_dialect::asm::AsmExtractor;
