//! Index-time concerns: structural facts extracted from source.
//! This module must not depend on public MCP/tool response shaping.

pub mod code_index;
pub mod doc_index;
pub mod indexer;

pub use indexer::Indexer;
