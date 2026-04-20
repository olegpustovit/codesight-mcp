//! SQL language folder.
//!
//! SQL's curated extraction is still provided by `sequel`-family grammars in
//! the shared extractor pipeline. What lives here today is framework code
//! that operates on top of a SQL host: dbt.

pub mod dbt;
