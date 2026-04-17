//! Public barrel for symbol types exercised by integration tests.
//! Internal code should use `crate::core::...`, `codesight_storage::storage::...`, and `crate::service::savings::...`.

pub use crate::core::pipeline::StoredSymbol;
pub use crate::core::symbols::{ExtractionTier, FactQuality, SymbolKind};
