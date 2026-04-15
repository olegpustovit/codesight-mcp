//! Multilingual import extraction + package-manifest discovery and registry.
//!
//! Tree-sitter-first import edge extraction with line-based fallbacks, plus
//! per-ecosystem package manifest parsing feeding the cross-repo package
//! registry. Consumed by analysis (`import_graph`, `dependencies`), indexing
//! (`code_index`), and MCP graph handlers.

pub mod imports;
pub mod packages;
