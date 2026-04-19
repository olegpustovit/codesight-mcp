//! codesight-engine: MCP tool dispatch, indexing orchestration, tool handlers.
//!
//! The engine owns the request/response lifecycle for the MCP tool surface
//! defined in `tools.json`. Backend-agnostic: structural facts come from a
//! `codesight-backend::CodeBackend` implementation chosen by the host binary.

pub mod agent_v1;
pub mod config;
pub mod indexing;
mod rpc;
pub mod security;
pub mod server_support;
pub mod service;

pub use server_support::{
    github_index, path_map, rate_limit, reindex_state, repo_id, result_cache,
};
pub use service::{CodeService, savings};
