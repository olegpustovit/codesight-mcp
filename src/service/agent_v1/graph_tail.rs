//! `inspect_graph` modes from dependency analysis onward.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::indexing::imports::{extract_imports, language_for_path, package_reference_evidence};
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::ToolError;
use crate::storage::{IndexStore, list_repo_summaries};

impl CodeService {
    pub(in crate::service) fn inspect_graph_tail(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        limit: usize,
        warnings: &mut Vec<Value>,
    ) -> anyhow::Result<Value> {
        match mode {
            "dependency_cycles" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let adj = store.dependency_graph()?;
                let cycles = crate::dep_graph::find_cycles(&adj);
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
            "dependencies" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let adj = store.dependency_graph()?;
                let edges: Vec<Value> = adj
                    .iter()
                    .flat_map(|(src, targets)| {
                        targets.iter().map(move |tgt| {
                            json!({
                                "relation": "depends_on",
                                "from_ref": { "path": src, "kind": "file" },
                                "to_ref": { "path": tgt, "kind": "file" },
                            })
                        })
                    })
                    .collect();
                let mut data = json!({
                    "graph": mode,
                    "edges": edges,
                    "nodes": adj.keys().collect::<Vec<_>>(),
                });

                // Cross-repo support
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
                    // Check this repo's files for references to other repos' packages
                    if let Ok(all_contents) = store.all_file_contents() {
                        for (file, content) in &all_contents {
                            let lang = language_for_path(file);
                            let edges = extract_imports(content, file, lang);
                            for (pkg_name, target_repos) in &registry {
                                if let Some(evidence) =
                                    package_reference_evidence(&edges, lang, pkg_name)
                                {
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
                        let key =
                            format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
                        seen.insert(key)
                    });
                    data["cross_repo_edges"] = json!(cross_repo_edges);
                }

                self.success_response(&timer, data)
            }
            "coupling_metrics" => {
                let module_path = match args.module_path.clone() {
                    Some(p) => p,
                    None => {
                        return self.error_response(
                            &MetaBuilder::new(0),
                            ToolError {
                                code: "INVALID_REQUEST",
                                message: "coupling_metrics requires module_path.".to_string(),
                                detail: json!({}),
                            },
                        );
                    }
                };
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                if !store.file_exists(&module_path)? {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!("File '{module_path}' not found in index."),
                            detail: json!({ "module_path": module_path }),
                        },
                    );
                }
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let adj = store.dependency_graph()?;
                let metrics = crate::dep_graph::compute_coupling(&module_path, &adj);
                self.success_response(
                    &timer,
                    json!({
                        "graph": mode,
                        "repo": args.source_id,
                        "module": metrics.module,
                        "ca": metrics.ca,
                        "ce": metrics.ce,
                        "instability": metrics.instability,
                        "assessment": metrics.assessment,
                        "importers": metrics.importers,
                        "dependencies": metrics.dependencies,
                    }),
                )
            }
            "layer_violations" => {
                let rules = args.rules.clone().unwrap_or_default();
                if rules.is_empty() {
                    let _store =
                        match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                            Ok(s) => s,
                            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                        };
                    return self.success_response(
                        &MetaBuilder::new(0),
                        json!({
                            "graph": mode,
                            "repo": args.source_id,
                            "layer_count": 0,
                            "violation_count": 0,
                            "violations": [],
                            "note": "No layer rules provided.",
                        }),
                    );
                }
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let adj = store.dependency_graph()?;
                let violations = crate::dep_graph::find_layer_violations(&adj, &rules);
                self.success_response(
                    &timer,
                    json!({
                        "graph": mode,
                        "repo": args.source_id,
                        "layer_count": rules.len(),
                        "violation_count": violations.len(),
                        "violations": violations,
                    }),
                )
            }
            "calls" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let Some(symbol_id) = args.symbol_id.as_deref() else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "calls mode requires symbol_id.".into(),
                            detail: json!({}),
                        },
                    );
                };

                // Load all symbols with source text
                let sym_infos = store.all_symbols_with_source()?;

                // Check the symbol exists
                let target = sym_infos.iter().find(|s| s.id == symbol_id);
                if target.is_none() {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!("Symbol '{symbol_id}' not found in index."),
                            detail: json!({ "symbol_id": symbol_id }),
                        },
                    );
                }
                let target = target.unwrap();

                let edges = crate::dep_graph::build_call_edges(&sym_infos);
                let direction = args.direction.as_deref().unwrap_or("both");
                let depth = args.depth.unwrap_or(3);

                let caller_tuples = if direction == "callees" {
                    vec![]
                } else {
                    crate::dep_graph::find_callers(symbol_id, &edges, depth)
                };
                let callee_tuples = if direction == "callers" {
                    vec![]
                } else {
                    crate::dep_graph::find_callees(symbol_id, &edges, depth)
                };

                // Convert tuples (id, name, file, line, kind, depth) to JSON
                let to_json =
                    |tuples: &[(String, String, String, u32, String, usize)]| -> Vec<Value> {
                        tuples
                            .iter()
                            .map(|(id, name, file, line, kind, depth)| {
                                json!({
                                    "id": id, "name": name, "kind": kind,
                                    "file": file, "line": line, "depth": depth,
                                })
                            })
                            .collect()
                    };

                let callers_json = to_json(&caller_tuples);
                let callees_json = to_json(&callee_tuples);
                let max_depth = caller_tuples
                    .iter()
                    .map(|t| t.5)
                    .chain(callee_tuples.iter().map(|t| t.5))
                    .max()
                    .unwrap_or(0);

                self.success_response(
                    &MetaBuilder::new(0),
                    json!({
                        "graph": "calls",
                        "symbol": {
                            "id": target.id,
                            "name": target.name,
                            "kind": target.kind,
                            "file": target.file,
                            "line": target.line,
                        },
                        "callers": callers_json,
                        "callees": callees_json,
                        "caller_count": callers_json.len(),
                        "callee_count": callees_json.len(),
                        "depth_reached": max_depth,
                        "_meta": { "source": "text_heuristic" },
                    }),
                )
            }
            "centrality" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let algo = args.algorithm.as_deref().unwrap_or("pagerank");
                if algo != "pagerank" && algo != "degree" {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: format!(
                                "Unknown centrality algorithm '{algo}'. Use 'pagerank' or 'degree'."
                            ),
                            detail: json!({ "algorithm": algo }),
                        },
                    );
                }

                let adj = store.dependency_graph()?;
                let degrees = crate::dep_graph::compute_in_out_degrees(&adj);
                let top_n = args.top_n.unwrap_or(20);

                let (ranked, iterations) = if algo == "pagerank" {
                    let pr = crate::dep_graph::compute_pagerank(&adj, 0.85, 100, 1e-6);
                    let mut ranked: Vec<_> = pr.scores.iter().collect();
                    ranked
                        .sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
                    let ranked: Vec<Value> = ranked
                        .iter()
                        .take(top_n)
                        .enumerate()
                        .map(|(i, (node, score))| {
                            let deg = degrees.get(*node);
                            json!({
                                "symbol_id": node,
                                "rank": i + 1,
                                "score": score,
                                "in_degree": deg.map(|d| d.in_degree).unwrap_or(0),
                                "out_degree": deg.map(|d| d.out_degree).unwrap_or(0),
                                "kind": "file",
                            })
                        })
                        .collect();
                    (ranked, pr.iterations)
                } else {
                    // degree-based: score = in_degree + out_degree
                    let mut ranked: Vec<_> = degrees.iter().collect();
                    ranked.sort_by(|a, b| {
                        (b.1.in_degree + b.1.out_degree).cmp(&(a.1.in_degree + a.1.out_degree))
                    });
                    let ranked: Vec<Value> = ranked
                        .iter()
                        .take(top_n)
                        .enumerate()
                        .map(|(i, (node, deg))| {
                            json!({
                                "symbol_id": node,
                                "rank": i + 1,
                                "score": (deg.in_degree + deg.out_degree) as f64,
                                "in_degree": deg.in_degree,
                                "out_degree": deg.out_degree,
                                "kind": "file",
                            })
                        })
                        .collect();
                    (ranked, 0)
                };

                self.success_response(
                    &MetaBuilder::new(0),
                    json!({
                        "graph": "centrality",
                        "algorithm": algo,
                        "ranked_symbols": ranked,
                        "iterations_to_converge": iterations,
                    }),
                )
            }
            "importers" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let Some(file_path) = args.file_path.as_deref() else {
                    return self.error_response(
                        &MetaBuilder::new(0),
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "importers mode requires file_path.".into(),
                            detail: json!({}),
                        },
                    );
                };
                let adj = store.dependency_graph()?;
                // Reverse lookup: find all files that have file_path in their deps
                let importers: Vec<&String> = adj
                    .iter()
                    .filter(|(_, deps)| deps.iter().any(|d| d == file_path))
                    .map(|(src, _)| src)
                    .collect();
                let mut data = json!({
                    "graph": "importers",
                    "file_path": file_path,
                    "importer_count": importers.len(),
                    "importers": importers,
                    "has_importers": !importers.is_empty(),
                });

                // Cross-repo support
                let cross_repo = args.cross_repo.unwrap_or(self.config.cross_repo_default);
                if cross_repo {
                    // Look for cross-repo importers
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
                    // Get package names for current repo
                    let my_pkg_names = store.get_package_names().unwrap_or_default();
                    let mut cross_importers: Vec<Value> = Vec::new();
                    // Check other repos' files for imports of our packages
                    for entry in &repo_entries {
                        if entry.repo_id == args.source_id {
                            continue;
                        }
                        let db_path = self.config.repo_db_path(&entry.repo_id);
                        if let Ok(other_store) = IndexStore::open(db_path)
                            && let Ok(root) = other_store.root_path()
                        {
                            let files = other_store.all_file_paths().unwrap_or_default();
                            for file in &files {
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
                    // Deduplicate
                    let mut seen: HashSet<String> = HashSet::new();
                    cross_importers.retain(|e| {
                        let key =
                            format!("{}->{}:{}", e["from_repo"], e["to_repo"], e["package_name"]);
                        seen.insert(key)
                    });
                    data["cross_repo_importer_count"] = json!(cross_importers.len());
                    data["cross_repo_importers"] = json!(cross_importers);
                }

                self.success_response(&MetaBuilder::new(0), data)
            }
            tail => self.inspect_graph_tail_b(args, tail, limit, warnings),
        }
    }
}
