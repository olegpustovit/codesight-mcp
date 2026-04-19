//! Build repos list and cross-repo import edges for `get_cross_repo_map`.

use std::collections::HashSet;

use serde_json::{Value, json};

use codesight_imports::{
    imports::{extract_imports, language_for_path, package_reference_evidence},
    packages,
};

use codesight_storage::storage::{IndexStore, list_repo_summaries};

use crate::config::AppConfig;
use crate::server_support::source_registry::SourceRegistry;

pub enum BuildCrossRepoOutcome {
    Found {
        repos: Vec<Value>,
        cross_repo_edges: Vec<Value>,
    },
    FilterMissing {
        source_id: String,
    },
}

/// Builds repo entries and cross-repo edges for `get_cross_repo_map`.
///
/// The `registry` is the process-wide per-source snapshot cache; each source's
/// `package_names` is read through the registry so repeated `get_cross_repo_map`
/// calls (and concurrent cross-source tools) skip per-repo SQLite opens when
/// the fingerprint hasn't moved.
pub fn build_cross_repo_map(
    config: &AppConfig,
    registry: &SourceRegistry,
    filter_id: Option<&str>,
) -> anyhow::Result<BuildCrossRepoOutcome> {
    let summaries = list_repo_summaries(&config.repos_dir())?;

    if let Some(filter_id) = filter_id {
        let found = summaries.iter().any(|s| s.repo == filter_id);
        if !found {
            return Ok(BuildCrossRepoOutcome::FilterMissing {
                source_id: filter_id.to_string(),
            });
        }
    }

    let mut repo_entries: Vec<packages::RepoEntry> = Vec::new();
    for summary in &summaries {
        let db_path = config.repo_db_path(&summary.repo);
        if db_path.exists()
            && let Ok(store) = IndexStore::open(db_path)
            && let Ok(snap) = registry.snapshot(&summary.repo, &store)
        {
            repo_entries.push(packages::RepoEntry {
                repo_id: summary.repo.clone(),
                package_names: snap.package_names.clone(),
            });
        }
    }

    let filtered_entries: Vec<&packages::RepoEntry> = if let Some(filter_id) = filter_id {
        repo_entries
            .iter()
            .filter(|e| e.repo_id == filter_id)
            .collect()
    } else {
        repo_entries.iter().collect()
    };

    let repos_json: Vec<Value> = filtered_entries
        .iter()
        .map(|e| {
            json!({
                "repo": e.repo_id,
                "package_names": e.package_names,
            })
        })
        .collect();

    let package_registry = packages::build_package_registry(&repo_entries);
    let mut cross_repo_edges: Vec<Value> = Vec::new();

    for entry in &filtered_entries {
        let db_path = config.repo_db_path(&entry.repo_id);
        // `all_file_contents()` is intentionally not cached in the registry —
        // source text for every file across every source would blow the
        // memory budget; the open + single scan stays per-call.
        if let Ok(store) = IndexStore::open(db_path)
            && let Ok(all_contents) = store.all_file_contents()
        {
            for (file, content) in &all_contents {
                let lang = language_for_path(file);
                let edges = extract_imports(content, file, lang);
                for (pkg_name, target_repos) in &package_registry {
                    if let Some(evidence) = package_reference_evidence(&edges, lang, pkg_name) {
                        for tr in target_repos {
                            if tr != &entry.repo_id {
                                cross_repo_edges.push(json!({
                                    "from_repo": entry.repo_id,
                                    "to_repo": tr,
                                    "package_name": pkg_name,
                                    "evidence": evidence,
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    let mut seen_edges: HashSet<String> = HashSet::new();
    cross_repo_edges.retain(|e| {
        let key = format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
        seen_edges.insert(key)
    });

    Ok(BuildCrossRepoOutcome::Found {
        repos: repos_json,
        cross_repo_edges,
    })
}
