//! Workspace-local `.dowser.toml` parsing and query-pack root discovery.
//!
//! Phase C moved ownership of `extra_extensions` / `query_pack_roots` out of
//! `dowser-extractors` into the language crate so the registry can build
//! a workspace-aware view via [`super::LanguageRegistry::for_workspace_root`].
//!
//! The schema is unchanged (`extra_extensions: Map<String, String>`,
//! `query_pack_roots: Vec<String>`) — this is a physical move, not a
//! redesign. Extractors still read query pack roots through
//! [`project_query_pack_roots`] to build SCM extractor overlays.

use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const PROJECT_CONFIG_FILE: &str = ".dowser.toml";

#[derive(Debug, Deserialize)]
pub struct DowserProjectConfig {
    #[serde(default)]
    pub extra_extensions: Option<FxHashMap<String, String>>,
    #[serde(default)]
    pub query_pack_roots: Option<Vec<String>>,
}

pub fn read_project_config(project_root: &Path) -> Option<DowserProjectConfig> {
    let content = std::fs::read_to_string(project_root.join(PROJECT_CONFIG_FILE)).ok()?;
    toml::from_str(&content).ok()
}

pub fn project_extra_extensions(project_root: &Path) -> FxHashMap<String, String> {
    read_project_config(project_root)
        .and_then(|cfg| cfg.extra_extensions)
        .unwrap_or_default()
}

/// Workspace-local query pack directories. Order:
///   1. Legacy `query_pack_roots` entries from `.dowser.toml`.
///   2. `<project_root>/.dowser/queries/` when present.
pub fn project_query_pack_roots(project_root: &Path) -> Vec<PathBuf> {
    let toml_roots: Vec<PathBuf> = read_project_config(project_root)
        .and_then(|cfg| cfg.query_pack_roots)
        .unwrap_or_default()
        .into_iter()
        .map(|path| {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                project_root.join(path)
            }
        })
        .collect();

    let mut combined = toml_roots;
    let convention = project_root.join(".dowser/queries");
    if convention.is_dir() {
        combined.push(convention);
    }
    dedupe_query_pack_roots(combined)
}

fn dedupe_query_pack_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(roots.len());
    let mut seen = FxHashSet::default();
    for root in roots {
        let key = root.canonicalize().unwrap_or_else(|_| root.clone());
        if seen.insert(key) {
            out.push(root);
        }
    }
    out
}
