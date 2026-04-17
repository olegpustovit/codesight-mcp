//! Compile-time defaults consumed across the crate.

pub const BYTES_PER_TOKEN: u64 = 4;
pub const RETRIEVAL_VERSION: &str = "agent-v1";
pub const RETRIEVAL_ENGINE: &str = "codesight";
pub const SCHEMA_VERSION: u32 = 2;

/// Short git SHA (12 hex chars) of the commit this binary was built from,
/// or `"unknown"` when the build ran outside a git checkout. Populated by
/// `build.rs` via `cargo:rustc-env=CODESIGHT_BUILD_SHA=...`.
pub const BUILD_SHA: &str = env!("CODESIGHT_BUILD_SHA");

/// Build timestamp as seconds since the Unix epoch (or the
/// `SOURCE_DATE_EPOCH` override for reproducible builds). Emitted on every
/// response so clients can detect a stale long-running MCP server whose
/// on-disk fix has already landed.
pub const BUILD_TIME: &str = env!("CODESIGHT_BUILD_TIME");

/// Semver package version (`env!("CARGO_PKG_VERSION")`).
pub const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Threshold for emitting a "No .gitignore" warning during ingest.
pub const GITIGNORE_WARNING_FILE_THRESHOLD: usize = 500;

/// Per-handler wall-clock timeout (milliseconds). Exceeding it yields a
/// categorized `TIMEOUT` error.
pub const HANDLER_TIMEOUT_MS: u64 = 2_000;

/// How long a shutdown drain waits for in-flight dispatches before
/// checkpointing and exiting anyway.
pub const SHUTDOWN_DRAIN_SECS: u64 = 5;

/// Bound on in-flight `tools/call` dispatches. Small enough to avoid SQLite
/// contention under watch mode or batch tools, large enough that quick
/// list-style reads don't queue behind a slow ingest.
pub const DISPATCH_CONCURRENCY: usize = 4;

/// Byte threshold above which whole-file reads emit `TOKEN_HEAVY_READ`.
pub const TOKEN_HEAVY_READ_BYTES: u64 = 4096;
