//! Generic MCP (Model Context Protocol) JSON-RPC 2.0 framing.
//!
//! Two halves, independent:
//!
//! - `server`: stdio JSON-RPC server. Accepts [`Dispatcher`] implementations,
//!   handles `initialize` / `tools/list` / `tools/call` / notifications, and
//!   drives a bounded-concurrency dispatch loop with graceful shutdown.
//!
//! - `client`: minimal stdio JSON-RPC client for spawning an MCP server as a
//!   child process and calling its tools.
//!
//! No `codesight_*` dependencies — the engine plugs in as a [`Dispatcher`].

pub mod client;
pub mod server;

pub use client::{McpClient, McpClientError};
pub use server::{DispatchConfig, Dispatcher, run_dispatch_loop, run_stdio, run_stdio_async};
