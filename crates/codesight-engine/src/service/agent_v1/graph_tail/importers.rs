//! `inspect_graph graph=importers` — who imports this file, plus optional cross-repo importers.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::agent_v1::util::path_matches_filter;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::{IndexStore, list_repo_summaries};
use codesight_language_support::imports_orchestration::{extract_imports, language_for_path};
use codesight_language_support::packages::package_reference_evidence;

impl CodeService {
    /// `graph=imports` — outgoing dependencies: what does this file import FROM?
    pub(super) fn graph_imports(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let Some(file_path) = args.file_path.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "imports mode requires file_path.".into(),
                    detail: json!({}),
                },
            );
        };
        let adj = store.dependency_graph()?;
        let imports: Vec<&String> = adj
            .get(file_path)
            .map(|deps| {
                deps.iter()
                    .filter(|target| {
                        path_matches_filter(
                            target,
                            args.scope.as_deref(),
                            args.exclude.as_deref(),
                            args.file_pattern.as_deref(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let data = json!({
            "graph": "imports",
            "file_path": file_path,
            "import_count": imports.len(),
            "imports": imports,
            "has_imports": !imports.is_empty(),
        });
        self.success_response(&MetaBuilder::new(0), data)
    }

    pub(super) fn graph_importers(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let Some(file_path) = args.file_path.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "importers mode requires file_path.".into(),
                    detail: json!({}),
                },
            );
        };
        let adj = store.dependency_graph()?;
        let importers: Vec<&String> = adj
            .iter()
            .filter(|(_, deps)| deps.iter().any(|d| d == file_path))
            .map(|(src, _)| src)
            .filter(|src| {
                path_matches_filter(
                    src,
                    args.scope.as_deref(),
                    args.exclude.as_deref(),
                    args.file_pattern.as_deref(),
                )
            })
            .collect();
        let mut data = json!({
            "graph": "importers",
            "file_path": file_path,
            "importer_count": importers.len(),
            "importers": importers,
            "has_importers": !importers.is_empty(),
        });

        let cross_repo = args.cross_repo.unwrap_or(false);
        if cross_repo {
            let summaries = list_repo_summaries(&self.config.repos_dir())?;
            let registry = self.source_registry();
            let mut repo_entries = Vec::new();
            for summary in &summaries {
                let db_path = self.config.repo_db_path(&summary.repo);
                if db_path.exists()
                    && let Ok(s) = IndexStore::open(db_path)
                    && let Ok(snap) = registry.snapshot(&summary.repo, &s)
                {
                    repo_entries.push(codesight_language_support::packages::RepoEntry {
                        repo_id: summary.repo.clone(),
                        package_names: snap.package_names.clone(),
                    });
                }
            }
            let my_pkg_names = registry
                .snapshot(&args.source_id, &store)
                .map(|snap| snap.package_names.clone())
                .unwrap_or_default();
            let mut cross_importers: Vec<Value> = Vec::new();
            for entry in &repo_entries {
                if entry.repo_id == args.source_id {
                    continue;
                }
                let db_path = self.config.repo_db_path(&entry.repo_id);
                if let Ok(other_store) = IndexStore::open(db_path)
                    && let Ok(other_snap) = registry.snapshot(&entry.repo_id, &other_store)
                {
                    let root = &other_snap.root_path;
                    for file in &other_snap.all_file_paths {
                        let full_path = root.join(file);
                        if let Ok(content) = std::fs::read_to_string(&full_path) {
                            let language = language_for_path(file);
                            let edges = extract_imports(&content, file, language);
                            for pkg in &my_pkg_names {
                                if let Some(evidence) =
                                    package_reference_evidence(&edges, language, pkg)
                                {
                                    cross_importers.push(json!({
                                        "from_repo": entry.repo_id,
                                        "to_repo": args.source_id,
                                        "package_name": pkg,
                                        "evidence": evidence,
                                    }));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let mut seen: HashSet<String> = HashSet::new();
            cross_importers.retain(|e| {
                let key = format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
                seen.insert(key)
            });
            data["cross_repo_importer_count"] = json!(cross_importers.len());
            data["cross_repo_importers"] = json!(cross_importers);
        }

        self.success_response(&MetaBuilder::new(0), data)
    }
}
