//! dowser-engine: MCP tool dispatch, indexing orchestration, tool handlers.
//!
//! The engine owns the request/response lifecycle for the MCP tool surface
//! defined in `tools.json`. Structural facts come from the tree-sitter-backed
//! [`indexing::Indexer`] which dispatches through `dowser-language-support`.

pub mod config;
pub mod indexing;
mod rpc;
pub mod security;
pub mod server_support;
pub mod service;

pub use server_support::{github_index, path_map, reindex_state, repo_id};
pub use service::{CodeService, savings};
