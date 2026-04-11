//! Precedence-ordered loader for [`AppConfig`].
//!
//! Precedence, highest wins:
//!   1. environment variables (`CODESIGHT_*`)
//!   2. global `~/.config/codesight/config.toml`
//!   3. compiled defaults
//!
//! Per-workspace `.codesight.toml` is read separately by the adaptive-language
//! flow, because its scope is *the workspace being ingested*, not the server's
//! global lifetime.

use std::path::PathBuf;

use serde::Deserialize;

use super::app::AppConfig;
use super::env;

#[derive(Debug, Default, Deserialize)]
struct GlobalConfigFile {
    #[serde(default)]
    auto_reindex: Option<bool>,
    #[serde(default)]
    index_dir: Option<PathBuf>,
    #[serde(default)]
    max_folder_files: Option<usize>,
    #[serde(default)]
    cross_repo_default: Option<bool>,
    #[serde(default)]
    strict_freshness: Option<bool>,
    #[serde(default)]
    handler_timeout_secs: Option<u64>,
    #[serde(default)]
    shutdown_drain_secs: Option<u64>,
    #[serde(default)]
    dispatch_concurrency: Option<usize>,
    #[serde(default)]
    trusted_folders: Option<Vec<PathBuf>>,
    #[serde(default)]
    trusted_folders_whitelist_mode: Option<bool>,
}

pub fn global_config_path() -> Option<PathBuf> {
    if let Some(p) = env::opt_path(env::GLOBAL_CONFIG) {
        return Some(p);
    }
    dirs::config_dir().map(|base| base.join("codesight").join("config.toml"))
}

fn read_global_file() -> GlobalConfigFile {
    let Some(path) = global_config_path() else {
        return GlobalConfigFile::default();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return GlobalConfigFile::default();
    };
    match toml::from_str::<GlobalConfigFile>(&content) {
        Ok(cfg) => {
            tracing::debug!(path = %path.display(), "loaded global codesight config");
            cfg
        }
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "global codesight config is malformed; falling back to defaults"
            );
            GlobalConfigFile::default()
        }
    }
}

fn default_index_root() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".code-index").join("codesight"))
        .unwrap_or_else(|| PathBuf::from(".code-index").join("codesight"))
}

pub fn load_from_env_and_disk() -> AppConfig {
    let file = read_global_file();

    let index_root = env::opt_path(env::INDEX_DIR)
        .or_else(|| env::opt_path(env::STORAGE_DIR))
        .or(file.index_dir)
        .unwrap_or_else(default_index_root);

    let mut config = AppConfig::new(index_root);

    config.auto_reindex = env::opt_bool(env::AUTO_REINDEX)
        .or(file.auto_reindex)
        .unwrap_or(true);

    config.max_folder_files = env::opt_usize(env::MAX_FOLDER_FILES)
        .or(file.max_folder_files)
        .unwrap_or(super::defaults::DEFAULT_MAX_FOLDER_FILES);

    config.cross_repo_default = env::opt_bool(env::CROSS_REPO_DEFAULT)
        .or(file.cross_repo_default)
        .unwrap_or(false);

    config.strict_freshness = env::opt_bool(env::STRICT_FRESHNESS)
        .or(file.strict_freshness)
        .unwrap_or(false);

    config.handler_timeout_secs = env::opt_u64(env::HANDLER_TIMEOUT_SECS)
        .or(file.handler_timeout_secs)
        .unwrap_or(super::defaults::DEFAULT_HANDLER_TIMEOUT_SECS);

    config.shutdown_drain_secs = env::opt_u64(env::SHUTDOWN_DRAIN_SECS)
        .or(file.shutdown_drain_secs)
        .unwrap_or(super::defaults::DEFAULT_SHUTDOWN_DRAIN_SECS);

    config.dispatch_concurrency = env::opt_usize(env::DISPATCH_CONCURRENCY)
        .or(file.dispatch_concurrency)
        .filter(|n| *n > 0)
        .unwrap_or(super::defaults::DEFAULT_DISPATCH_CONCURRENCY);

    if let Some(folders) = file.trusted_folders {
        config.trusted_folders = folders;
    }
    if let Some(mode) = file.trusted_folders_whitelist_mode {
        config.trusted_folders_whitelist_mode = mode;
    }

    config.path_map = crate::path_map::parse_path_map_from_env();
    config
}
