//! Compile-time defaults consumed across the crate.

/// Byte → token conversion heuristic used for budgeting serialized response
/// sizes. This is an **estimator**, not a tokenizer: we do not ship a BPE
/// model with the server, and the correct tokenizer varies per consumer
/// LLM (Claude ≠ GPT). 4 bytes/token is the industry heuristic for English
/// source text; it is accurate enough for truncation decisions and avoids
/// the per-byte overhead plus per-model coupling a real tokenizer would
/// introduce. Budgets that require exact counts belong in the agent.
pub const BYTES_PER_TOKEN: u64 = 4;
pub const RETRIEVAL_VERSION: &str = "agent-v1";
pub const RETRIEVAL_ENGINE: &str = "dowser";
pub const SCHEMA_VERSION: u32 = 2;

/// Short git SHA (12 hex chars) of the commit this binary was built from,
/// or `"unknown"` when the build ran outside a git checkout. Populated by
/// `build.rs` via `cargo:rustc-env=DOWSER_BUILD_SHA=...`.
pub const BUILD_SHA: &str = env!("DOWSER_BUILD_SHA");

/// Build timestamp as seconds since the Unix epoch (or the
/// `SOURCE_DATE_EPOCH` override for reproducible builds). Emitted on every
/// response so clients can detect a stale long-running MCP server whose
/// on-disk fix has already landed.
pub const BUILD_TIME: &str = env!("DOWSER_BUILD_TIME");

/// Semver package version (`env!("CARGO_PKG_VERSION")`).
pub const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Threshold for emitting a "No .gitignore" warning during ingest.
pub const GITIGNORE_WARNING_FILE_THRESHOLD: usize = 500;

/// How long a shutdown drain waits for in-flight dispatches before
/// checkpointing and exiting anyway.
pub const SHUTDOWN_DRAIN_SECS: u64 = 5;

/// Bound on in-flight `tools/call` dispatches. Small enough to avoid SQLite
/// contention under watch mode or batch tools, large enough that quick
/// list-style reads don't queue behind a slow ingest.
pub const DISPATCH_CONCURRENCY: usize = 4;

/// Byte threshold above which whole-file reads emit `TOKEN_HEAVY_READ`.
pub const TOKEN_HEAVY_READ_BYTES: u64 = 4096;
