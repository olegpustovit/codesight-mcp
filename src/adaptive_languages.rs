//! Adaptive language detection: TOML project config for enabling languages per workspace.
//!
//! The narrow extractor-facing schema (`PROJECT_CONFIG_FILE`, `project_extra_extensions`,
//! `project_query_pack_roots`, `read_project_config`) lives in
//! [`codesight_extractors::adaptive_languages`] and is re-exported here so the product
//! crate has a single source of truth. The product-owned pieces below — in-memory config
//! cache and the full adaptive-language flow — add on top and require the extended
//! `CodesightProjectConfig` struct with `languages_adaptive` / `languages` fields.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use serde::Deserialize;

pub use codesight_extractors::adaptive_languages::{
    PROJECT_CONFIG_FILE, project_extra_extensions, project_query_pack_roots,
};

// ---------------------------------------------------------------------------
// Project config cache
// ---------------------------------------------------------------------------
//
// Read-dominant: invalidate/prime fire on config file changes; lookups happen
// on every indexing pass. `RwLock` lets parallel readers proceed without
// serializing behind a single mutex guard.

static PROJECT_CONFIGS: LazyLock<RwLock<HashMap<String, serde_json::Value>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

static PROJECT_CONFIG_HASHES: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Evict a project root from the in-memory config caches.
pub fn invalidate_project_config_cache(project_root: &str) {
    let resolved = resolve_path(project_root);
    if let Ok(mut configs) = PROJECT_CONFIGS.write() {
        configs.remove(&resolved);
    }
    if let Ok(mut hashes) = PROJECT_CONFIG_HASHES.write() {
        hashes.remove(&resolved);
    }
}

/// Prime the cache (test helper).
#[doc(hidden)]
pub fn prime_config_cache(project_root: &str, config: serde_json::Value, hash: &str) {
    let resolved = resolve_path(project_root);
    if let Ok(mut configs) = PROJECT_CONFIGS.write() {
        configs.insert(resolved.clone(), config);
    }
    if let Ok(mut hashes) = PROJECT_CONFIG_HASHES.write() {
        hashes.insert(resolved, hash.to_owned());
    }
}

/// Check whether a resolved key is present in the config cache.
#[doc(hidden)]
pub fn config_cache_contains(project_root: &str) -> (bool, bool) {
    let resolved = resolve_path(project_root);
    let in_configs = PROJECT_CONFIGS
        .read()
        .map(|c| c.contains_key(&resolved))
        .unwrap_or(false);
    let in_hashes = PROJECT_CONFIG_HASHES
        .read()
        .map(|h| h.contains_key(&resolved))
        .unwrap_or(false);
    (in_configs, in_hashes)
}

// ---------------------------------------------------------------------------
// TOML project shape (workspace `.codesight.toml`)
// ---------------------------------------------------------------------------

/// Product-layer view of `.codesight.toml`.
///
/// Adds `languages_adaptive` / `languages` on top of the narrow extractor-facing schema
/// in [`codesight_extractors::adaptive_languages::CodesightProjectConfig`]. The extractor
/// crate owns the extension/query-pack fields; this struct adds what the adaptive-language
/// flow below needs.
#[derive(Debug, Deserialize)]
pub struct CodesightProjectConfig {
    #[serde(default)]
    pub languages_adaptive: bool,
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    #[serde(default)]
    pub extra_extensions: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query_pack_roots: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// parse_active_languages
// ---------------------------------------------------------------------------

/// Parse `.codesight.toml` content and return active language names from the `languages` array.
///
/// Returns `None` if TOML is invalid, the `languages` key is absent, or its value is `null`
/// (represented in TOML by omitting the key — there is no TOML null).
pub fn parse_active_languages(content: &str) -> Option<HashSet<String>> {
    let cfg: CodesightProjectConfig = toml::from_str(content).ok()?;
    let arr = cfg.languages?;
    Some(arr.into_iter().collect())
}

// ---------------------------------------------------------------------------
// build_languages_block
// ---------------------------------------------------------------------------

/// Builds the `languages` array for the config file: **only** languages in `detected` are
/// listed (sorted). All other registered languages are omitted (disabled).
pub fn build_languages_block(detected: &HashSet<String>, _all_languages: &[&str]) -> String {
    let mut langs: Vec<&str> = detected.iter().map(|s| s.as_str()).collect();
    langs.sort();

    let mut lines = vec!["languages = [".to_string()];
    for lang in &langs {
        lines.push(format!("    \"{lang}\","));
    }
    lines.push("]".to_string());
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// check_raw_local_adaptive
// ---------------------------------------------------------------------------

/// Read a local `.codesight.toml` and check whether `languages_adaptive` is `true`.
///
/// Returns `(is_adaptive, raw_content)`. On missing file, invalid TOML, or I/O error,
/// returns `(false, "")`.
pub fn check_raw_local_adaptive(config_path: &Path) -> (bool, String) {
    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return (false, String::new()),
    };

    let Ok(cfg) = toml::from_str::<CodesightProjectConfig>(&content) else {
        return (false, String::new());
    };

    (cfg.languages_adaptive, content)
}

// ---------------------------------------------------------------------------
// apply_adaptive_languages
// ---------------------------------------------------------------------------

/// Full adaptive language update flow.
///
/// - If no local config exists and the global config has `languages_adaptive: true`,
///   creates a new `.codesight.toml` with detected languages.
/// - If the local config has `languages_adaptive: true`, updates `languages` when the set differs.
/// - If `languages_adaptive` is false, no-ops.
///
/// Returns `true` if the config file was created or updated; `false` otherwise.
pub fn apply_adaptive_languages(
    project_root: &str,
    detected: &HashSet<String>,
    global_adaptive: bool,
    _all_languages: &[&str],
) -> bool {
    let root = PathBuf::from(project_root);
    let config_path = root.join(PROJECT_CONFIG_FILE);

    if config_path.exists() {
        let (is_adaptive, content) = check_raw_local_adaptive(&config_path);
        if !is_adaptive {
            return false;
        }

        let existing = parse_active_languages(&content);
        let existing_set = existing.unwrap_or_default();

        if existing_set == *detected {
            return false;
        }

        let new_content = render_adaptive_config_toml(detected, _all_languages);
        if let Err(e) = std::fs::write(&config_path, &new_content) {
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "adaptive_languages: failed to write config"
            );
            return false;
        }

        invalidate_project_config_cache(project_root);
        true
    } else if global_adaptive {
        let new_content = render_adaptive_config_toml(detected, _all_languages);
        if let Err(e) = std::fs::write(&config_path, &new_content) {
            tracing::warn!(
                path = %config_path.display(),
                error = %e,
                "adaptive_languages: failed to create config"
            );
            return false;
        }

        invalidate_project_config_cache(project_root);
        true
    } else {
        false
    }
}

fn render_adaptive_config_toml(detected: &HashSet<String>, all_languages: &[&str]) -> String {
    let languages = build_languages_block(detected, all_languages);
    format!(
        "# codesight-mcp project configuration (TOML)\n\
         # Languages listed under `languages` are enabled when `languages_adaptive` is true.\n\
         \n\
         languages_adaptive = true\n\
         \n\
         {languages}\n"
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn resolve_path(p: &str) -> String {
    std::fs::canonicalize(p)
        .map(|c| c.to_string_lossy().to_string())
        .unwrap_or_else(|_| p.to_string())
}
