//! Host-side helpers: repo identity, path remapping, GitHub URL parsing, rate limiting,
//! tool result caching, and reindex lifecycle — kept out of MCP dispatch and indexing domains.

pub mod github_index;
pub mod path_map;
pub mod rate_limit;
pub mod reindex_state;
pub mod repo_id;
pub mod result_cache;
