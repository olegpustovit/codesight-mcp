//! Generic MCP (Model Context Protocol) JSON-RPC 2.0 framing.
//!
//! Stdio JSON-RPC server. Accepts [`Dispatcher`] implementations, handles
//! `initialize` / `tools/list` / `tools/call` / notifications, and drives a
//! bounded-concurrency dispatch loop with graceful shutdown.
//!
//! No `codesight_*` dependencies — the engine plugs in as a [`Dispatcher`].

pub mod server;

pub use server::{DispatchConfig, Dispatcher, run_dispatch_loop, run_stdio, run_stdio_async};
