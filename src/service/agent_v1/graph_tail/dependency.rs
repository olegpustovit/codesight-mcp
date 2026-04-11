//! `inspect_graph` dependency modes: cycles + forward edges (with optional cross-repo edges).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::indexing::imports::{extract_imports, language_for_path, package_reference_evidence};
use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::savings::MetaBuilder;
use crate::storage::{IndexStore, list_repo_summaries};

impl CodeService {
    pub(super) fn graph_dependency_cycles(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };
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
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let adj = store.dependency_graph_with_import_quality()?;
        let edges: Vec<Value> = adj
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
        let mut data = json!({
            "graph": mode,
            "edges": edges,
            "nodes": adj.keys().collect::<Vec<_>>(),
        });

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
                    repo_entries.push(crate::indexing::packages::RepoEntry {
                        repo_id: summary.repo.clone(),
                        package_names: pkg_names,
                    });
                }
            }
            let registry = crate::indexing::packages::build_package_registry(&repo_entries);
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
