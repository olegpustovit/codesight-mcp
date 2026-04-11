//! Product configuration.
//!
//! Layering:
//! - [`env`]: canonical list of `CODESIGHT_*` environment variables.
//! - [`defaults`]: compile-time constants (retrieval version, thresholds).
//! - [`app`]: the [`AppConfig`] struct handed down from `main`.
//! - [`loader`]: precedence-ordered loader (env → global toml → default).
//! - [`trust`]: path-trust policy enforced before indexing.
//! - [`paths`]: internal path-slug helpers.

pub mod defaults;
pub mod env;
pub mod loader;
pub mod trust;

mod app;
mod paths;

pub use app::AppConfig;
pub use defaults::{
    BUILD_SHA, BUILD_TIME, BUILD_VERSION, BYTES_PER_TOKEN, DEFAULT_MAX_FOLDER_FILES,
    GITIGNORE_WARNING_FILE_THRESHOLD, RETRIEVAL_ENGINE, RETRIEVAL_VERSION, SCHEMA_VERSION,
};
pub use trust::{TrustCheckResult, check_trust, is_broad_path, sanitize_project_trusted_folders};
