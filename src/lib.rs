pub mod adaptive_languages;
pub mod agent_v1;
pub mod cli;
pub mod config;
pub mod framework;
pub mod indexing;
pub mod mcp;
pub mod model;
pub mod retrieval;
pub mod security;
pub mod server_support;
pub mod service;
pub mod storage;
/// Synthetic repos and checked-in fixture paths for integration tests.
pub mod test_support;

pub use codesight_core as core;
pub use codesight_extractors::extractors;
pub use codesight_storage::db;

// Stable crate-root paths for integration tests and embedders (WS17 composition).
pub use server_support::{
    github_index, path_map, rate_limit, reindex_state, repo_id, result_cache,
};
pub use service::savings;
