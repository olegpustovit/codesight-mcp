//! `inspect_graph` dependency modes: cycles + forward edges (with optional cross-repo edges).

use std::collections::HashSet;

use serde_json::{Value, json};

use codesight_imports::imports::{extract_imports, language_for_path, package_reference_evidence};
use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use crate::storage::{IndexStore, list_repo_summaries};

impl CodeService {
    pub(super) fn graph_dependency_cycles(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let adj = store.dependency_graph_strict()?;
        let cycles = crate::analysis::graph::find_cycles(&adj);
        self.success_response(
            &timer,
            json!({
                "graph": mode,
                "repo": args.source_id,
                "cycle_count": cycles.len(),
                "cycles": cycles,
            }),
        )
    }

    pub(super) fn graph_dependencies(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let adj = store.dependency_graph_with_import_quality()?;
        let all_edges: Vec<Value> = adj
            .iter()
            .flat_map(|(src, targets)| {
                targets.iter().map(move |(tgt, quality)| {
                    json!({
                        "relation": "depends_on",
                        "from_ref": { "path": src, "kind": "file" },
                        "to_ref": { "path": tgt, "kind": "file" },
                        "import_fact_quality": quality.as_str(),
                    })
                })
            })
            .collect();
        let total_edges = all_edges.len();

        // Always apply a token budget (default 8k) plus an optional edge-count
        // cap. Without a default budget, large repos dumped the full `nodes`
        // array (hundreds of file paths) regardless of `limit`, which only
        // capped edges — a 400-file repo burned ~4k tokens on node lists.
        const DEFAULT_DEPENDENCIES_TOKEN_BUDGET: u32 = 8000;
        let mut capped_edges = all_edges;
        let mut edge_cap_truncated = false;
        if args.limit > 0 && capped_edges.len() > args.limit {
            capped_edges.truncate(args.limit);
            edge_cap_truncated = true;
        }
        let budget = args.max_tokens.unwrap_or(DEFAULT_DEPENDENCIES_TOKEN_BUDGET);
        let budgeted = true;
        let (edges, budget_truncated) =
            crate::service::agent_v1::util::pack_catalog_items_to_token_budget(
                capped_edges,
                budget,
            );
        let truncated = edge_cap_truncated || budget_truncated;

        let nodes: Vec<&String> = adj.keys().collect();
        let node_count = nodes.len();
        let mut data = if budgeted {
            // Under an explicit token budget, the nodes list is the biggest
            // single offender on a 400-file repo (~20k chars). Drop it and
            // return just a count; callers can retrieve the file list via
            // `inspect_structure view=file_tree` if they need it.
            json!({
                "graph": mode,
                "edges": edges,
                "edge_count": total_edges,
                "node_count": node_count,
                "truncated": truncated,
            })
        } else {
            json!({
                "graph": mode,
                "edges": edges,
                "edge_count": total_edges,
                "nodes": nodes,
                "node_count": node_count,
                "truncated": truncated,
            })
        };

        let cross_repo = args.cross_repo.unwrap_or(self.config.cross_repo_default);
        if cross_repo {
            let summaries = list_repo_summaries(&self.config.repos_dir())?;
            let mut repo_entries = Vec::new();
            for summary in &summaries {
                let db_path = self.config.repo_db_path(&summary.repo);
                if db_path.exists()
                    && let Ok(s) = IndexStore::open(db_path)
                {
                    let pkg_names = s.get_package_names().unwrap_or_default();
                    repo_entries.push(codesight_imports::packages::RepoEntry {
                        repo_id: summary.repo.clone(),
                        package_names: pkg_names,
                    });
                }
            }
            let registry = codesight_imports::packages::build_package_registry(&repo_entries);
            let mut cross_repo_edges: Vec<Value> = Vec::new();
            if let Ok(all_contents) = store.all_file_contents() {
                for (file, content) in &all_contents {
                    let lang = language_for_path(file);
                    let edges = extract_imports(content, file, lang);
                    for (pkg_name, target_repos) in &registry {
                        if let Some(evidence) = package_reference_evidence(&edges, lang, pkg_name) {
                            for tr in target_repos {
                                if tr != &args.source_id {
                                    cross_repo_edges.push(json!({
                                        "from_repo": args.source_id,
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
            let mut seen: HashSet<String> = HashSet::new();
            cross_repo_edges.retain(|e| {
                let key = format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
                seen.insert(key)
            });
            data["cross_repo_edges"] = json!(cross_repo_edges);
        }

        self.success_response(&timer, data)
    }
}
