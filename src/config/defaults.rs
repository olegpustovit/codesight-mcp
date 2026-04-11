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

/// Maximum number of files to index from a local folder (0 = unlimited).
pub const DEFAULT_MAX_FOLDER_FILES: usize = 0;

/// Default per-handler timeout (seconds). Applied to every `tools/call`
/// dispatch; exceeding it yields `ToolError::Transient` with code `TIMEOUT`.
pub const DEFAULT_HANDLER_TIMEOUT_SECS: u64 = 60;

/// Default bound on how long a shutdown drain waits for an in-flight
/// dispatch to finish before we checkpoint and exit anyway.
pub const DEFAULT_SHUTDOWN_DRAIN_SECS: u64 = 5;

/// Default bound on in-flight `tools/call` dispatches. Small enough to avoid
/// SQLite contention under watch mode or batch tools, large enough that quick
/// list-style reads don't queue behind a slow ingest.
pub const DEFAULT_DISPATCH_CONCURRENCY: usize = 4;
