//! Single source of truth for `CODESIGHT_*` environment variables.
//!
//! Every environment variable the product crate reads lives here. Consumers
//! call typed helpers instead of sprinkling `std::env::var` strings.

use std::path::PathBuf;

pub const AUTO_REINDEX: &str = "CODESIGHT_AUTO_REINDEX";
pub const INDEX_DIR: &str = "CODESIGHT_INDEX_DIR";
pub const STORAGE_DIR: &str = "CODESIGHT_STORAGE_DIR";
pub const MAX_FOLDER_FILES: &str = "CODESIGHT_MAX_FOLDER_FILES";
pub const CROSS_REPO_DEFAULT: &str = "CODESIGHT_CROSS_REPO_DEFAULT";
pub const STRICT_FRESHNESS: &str = "CODESIGHT_STRICT_FRESHNESS";
pub const LOG_FORMAT: &str = "CODESIGHT_LOG_FORMAT";
pub const PATH_MAP: &str = "CODESIGHT_PATH_MAP";
pub const GLOBAL_CONFIG: &str = "CODESIGHT_GLOBAL_CONFIG";
pub const SUMMARIZER_PROVIDER: &str = "CODESIGHT_SUMMARIZER_PROVIDER";
pub const SUMMARIZER_MODEL: &str = "CODESIGHT_SUMMARIZER_MODEL";
pub const HANDLER_TIMEOUT_SECS: &str = "CODESIGHT_HANDLER_TIMEOUT_SECS";
pub const SHUTDOWN_DRAIN_SECS: &str = "CODESIGHT_SHUTDOWN_DRAIN_SECS";
pub const DISPATCH_CONCURRENCY: &str = "CODESIGHT_DISPATCH_CONCURRENCY";

pub fn bool_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(v.as_str(), "false" | "0" | ""),
        Err(_) => default,
    }
}

pub fn opt_bool(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .map(|v| matches!(v.as_str(), "true" | "1"))
}

pub fn opt_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

pub fn opt_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

pub fn opt_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}
