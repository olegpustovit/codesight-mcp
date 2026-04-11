//! Compile-time defaults consumed across the crate.

pub const BYTES_PER_TOKEN: u64 = 4;
pub const RETRIEVAL_VERSION: &str = "agent-v1";
pub const RETRIEVAL_ENGINE: &str = "codesight";
pub const SCHEMA_VERSION: u32 = 2;

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
