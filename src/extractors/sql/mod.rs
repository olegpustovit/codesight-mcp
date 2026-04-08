//! Residual extractor facade.
//!
//! Category: `host_semantics`.
//! Exit criteria: delete this facade once dbt/Jinja host constructs and SQL fallback coverage can
//! be expressed through shared container/host extraction without SQL-specific Rust glue.

pub use crate::extractors::residual::host_semantics::sql::SqlExtractor;
