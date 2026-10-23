//! `inspect_graph graph=importers` — who imports this file, plus optional cross-repo importers.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::agent_v1::util::path_matches_filter;
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use dowser_language_support::imports_orchestration::{extract_imports, language_for_path};
use dowser_language_support::packages::package_reference_evidence;
use dowser_storage::storage::{IndexStore, list_repo_summaries};

impl CodeService {
    /// `graph=imports` — outgoing dependencies: what does this file import FROM?
    pub(super) fn graph_imports(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let file_path = match self.import_graph_path(&store, args) {
            Ok(path) => path,
            Err(error) => return self.error_response(&MetaBuilder::new(0), error),
        };
        let adj = store.dependency_graph_with_import_quality()?;
        let mut resolved = BTreeSet::new();
        let mut candidates = BTreeSet::new();
        for (target, quality) in adj.get(file_path).into_iter().flatten() {
            if path_matches_filter(
                target,
                args.scope.as_deref(),
                args.exclude.as_deref(),
                args.file_pattern.as_deref(),
            ) {
                if quality.suitable_for_strict_analysis() {
                    resolved.insert(target);
                } else {
                    candidates.insert(target);
                }
            }
        }
        let limit = args.limit.unwrap_or(50);
        let imports: Vec<_> = resolved.iter().take(limit).collect();
        let candidate_imports: Vec<_> = candidates
            .iter()
            .take(limit.saturating_sub(imports.len()))
            .collect();
        let truncated = resolved.len() + candidates.len() > imports.len() + candidate_imports.len();
        let data = json!({
            "graph": "imports",
            "file_path": file_path,
            "import_count": imports.len(),
            "total_imports": resolved.len(),
            "imports": imports,
            "has_imports": !resolved.is_empty(),
            "evidence": "structural_import_resolution", "complete": false,
            "candidate_imports": candidate_imports, "total_candidate_imports": candidates.len(),
            "candidate_evidence": "heuristic_import_resolution", "truncated": truncated,
        });
        let warnings = truncated
            .then_some(json!({"code":"TRUNCATED", "message":"imports capped by requested limit"}))
            .into_iter()
            .collect();
        self.success_response_with_warnings(
            &MetaBuilder::new(0).with_confidence(Confidence::Partial),
            data,
            warnings,
        )
    }

    pub(super) fn graph_importers(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let file_path = match self.import_graph_path(&store, args) {
            Ok(path) => path,
            Err(error) => return self.error_response(&MetaBuilder::new(0), error),
        };
        // Apply one incoming-edge predicate to both graphs, then separate evidence by set difference.
        let incoming = |graph: FxHashMap<String, Vec<String>>| -> BTreeSet<String> {
            graph
                .into_iter()
                .filter(|(src, deps)| {
                    deps.iter().any(|dep| dep == file_path)
                        && path_matches_filter(
                            src,
                            args.scope.as_deref(),
                            args.exclude.as_deref(),
                            args.file_pattern.as_deref(),
                        )
                })
                .map(|(src, _)| src)
                .collect()
        };
        let resolved = incoming(store.dependency_graph_strict()?);
        let usage = incoming(dowser_analysis::graph::adjacency_with_usage_edges(&store)?);
        let all_candidates: Vec<_> = usage.difference(&resolved).collect();
        let total = resolved.len();
        let limit = args.limit.unwrap_or(50);
        let importers: Vec<_> = resolved.iter().take(limit).collect();
        let candidates: Vec<_> = all_candidates
            .iter()
            .take(limit.saturating_sub(importers.len()))
            .collect();
        let candidate_total = all_candidates.len();
        let truncated = total + candidate_total > importers.len() + candidates.len();
        let mut data = json!({
            "graph": "importers", "file_path": file_path,
            "importer_count": importers.len(), "total_importers": total,
            "importers": importers, "has_importers": total > 0,
            "truncated": truncated,
            "evidence": "resolved_import",
            "complete": false,
        });

        data["candidate_usage_importers"] = json!(candidates);
        data["total_candidate_usage_importers"] = json!(candidate_total);
        data["candidate_evidence"] = json!("heuristic_import_resolution_or_name_usage");
        let cross_repo = args.cross_repo.unwrap_or(false);
        if cross_repo {
            let summaries = list_repo_summaries(&self.config.repos_dir())?;
            let registry = self.source_registry();
            let my_pkg_names = registry
                .snapshot(&args.source_id, &store)
                .map(|snap| snap.package_names.clone())
                .unwrap_or_default();
            let mut cross_importers: Vec<Value> = Vec::new();
            for summary in summaries.iter().filter(|s| s.repo != args.source_id) {
                let db_path = self.config.repo_db_path(&summary.repo);
                if let Some(other_snap) = IndexStore::open_existing(db_path)
                    .ok()
                    .flatten()
                    .and_then(|store| registry.snapshot(&summary.repo, &store).ok())
                {
                    for (file, content) in other_snap.all_file_paths.iter().filter_map(|file| {
                        std::fs::read_to_string(other_snap.root_path.join(file))
                            .ok()
                            .map(|content| (file, content))
                    }) {
                        let language = language_for_path(file);
                        let edges = extract_imports(&content, file, language);
                        if let Some((pkg, evidence)) = my_pkg_names.iter().find_map(|pkg| {
                            package_reference_evidence(&edges, language, pkg)
                                .map(|evidence| (pkg, evidence))
                        }) {
                            cross_importers.push(json!({
                                "from_repo": summary.repo, "to_repo": args.source_id,
                                "package_name": pkg, "evidence": evidence,
                            }));
                        }
                    }
                }
            }
            let mut seen: FxHashSet<String> = FxHashSet::default();
            cross_importers.retain(|e| {
                let key = format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
                seen.insert(key)
            });
            data["cross_repo_importer_count"] = json!(cross_importers.len());
            data["cross_repo_importers"] = json!(cross_importers);
        }

        let warnings = truncated
            .then_some(json!({
                "code": "TRUNCATED", "message": "importers capped by requested limit",
            }))
            .into_iter()
            .collect();
        self.success_response_with_warnings(
            &MetaBuilder::new(0).with_confidence(Confidence::Partial),
            data,
            warnings,
        )
    }
}
