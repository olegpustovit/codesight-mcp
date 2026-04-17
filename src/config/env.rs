//! Single source of truth for `CODESIGHT_*` environment variables.
//!
//! Every environment variable the product crate reads lives here. Consumers
//! call typed helpers instead of sprinkling `std::env::var` strings.

use std::path::PathBuf;

pub const INDEX_DIR: &str = "CODESIGHT_INDEX_DIR";
pub const LOG_FORMAT: &str = "CODESIGHT_LOG_FORMAT";
pub const PATH_MAP: &str = "CODESIGHT_PATH_MAP";

pub fn opt_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}
