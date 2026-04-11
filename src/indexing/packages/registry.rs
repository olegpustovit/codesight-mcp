//! In-memory package registry and cross-repo edge materialization.

use std::collections::{HashMap, HashSet};

use super::extract_root_package_from_specifier;

/// A mapping from package name -> list of repo IDs that publish it.
pub type PackageRegistry = HashMap<String, Vec<String>>;

/// Repo entry for building the package registry.
#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub repo_id: String,
    pub package_names: Vec<String>,
}

/// Build a registry mapping package names to the repo IDs that publish them.
pub fn build_package_registry(repos: &[RepoEntry]) -> PackageRegistry {
    let mut registry = PackageRegistry::new();
    for entry in repos {
        for name in &entry.package_names {
            registry
                .entry(name.clone())
                .or_default()
                .push(entry.repo_id.clone());
        }
    }
    registry
}

/// Find repo IDs that publish a given package name.
pub fn find_repos_for_package(package: &str, repos: &[RepoEntry]) -> Vec<String> {
    let registry = build_package_registry(repos);
    registry.get(package).cloned().unwrap_or_default()
}

/// Cross-repo edge in the dependency map.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CrossRepoEdge {
    pub from_repo: String,
    pub to_repo: String,
    pub package_name: String,
}

/// Build a cross-repo dependency map for all indexed repos.
///
/// For each repo, look at its imported packages, find which other repos
/// publish those packages, and create edges.
pub fn build_cross_repo_edges(
    repos: &[RepoEntry],
    repo_imports: &HashMap<String, Vec<String>>,
) -> Vec<CrossRepoEdge> {
    let registry = build_package_registry(repos);
    let mut edges = Vec::new();
    let mut seen = HashSet::new();

    for (consumer_repo, import_specifiers) in repo_imports {
        for specifier in import_specifiers {
            let root_candidates: Vec<String> = vec![
                extract_root_package_from_specifier(specifier, "python"),
                extract_root_package_from_specifier(specifier, "javascript"),
                extract_root_package_from_specifier(specifier, "go"),
                extract_root_package_from_specifier(specifier, "rust"),
            ];

            for root_pkg in &root_candidates {
                if root_pkg.is_empty() {
                    continue;
                }
                if let Some(provider_repos) = registry.get(root_pkg) {
                    for provider_repo in provider_repos {
                        if provider_repo == consumer_repo {
                            continue;
                        }
                        let key = (
                            consumer_repo.clone(),
                            provider_repo.clone(),
                            root_pkg.clone(),
                        );
                        if seen.insert(key) {
                            edges.push(CrossRepoEdge {
                                from_repo: consumer_repo.clone(),
                                to_repo: provider_repo.clone(),
                                package_name: root_pkg.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    edges
}
