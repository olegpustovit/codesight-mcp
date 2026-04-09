//! Security hardening utilities.

mod binary_policy;
mod config;
mod discovery;
mod encoding;
mod path_validation;
mod secret_policy;

pub use binary_policy::{
    ExcludeOptions, ExcludeReason, is_binary_content, is_binary_extension, is_binary_file,
    should_exclude_file,
};
pub use config::{
    DEFAULT_MAX_FILE_SIZE, DEFAULT_MAX_FOLDER_FILES, DEFAULT_MAX_INDEX_FILES, SecurityConfig,
    get_extra_ignore_patterns, get_max_folder_files, get_max_index_files,
};
pub use discovery::{DiscoverOptions, DiscoverResult, DiscoverWarning, discover_files};
pub use encoding::safe_decode;
pub use path_validation::{is_symlink_escape, validate_path};
pub use secret_policy::is_secret_file;
