//! Residual Rust extractors that remain after query-pack, generic-AST, and container-first paths.
//!
//! New language support should land in query packs, generic AST, or shared container helpers first.
//! This area is for the explicit remainder only.

pub(crate) mod container_boundary;
pub(crate) mod host_semantics;
pub(crate) mod multi_dialect;
pub(crate) mod no_grammar;
