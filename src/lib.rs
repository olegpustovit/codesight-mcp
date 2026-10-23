//! symslice-mcp binary crate: CLI subcommands + stdio MCP server entry point.
//! Business logic lives in `symslice-engine`. JSON-RPC framing lives in
//! `mcp-rpc`.

pub mod cli;
/// Synthetic repos and checked-in fixture paths for integration tests.
pub mod test_support;
