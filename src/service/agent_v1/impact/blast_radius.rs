//! `evaluate_impact analysis=blast_radius` — reverse-import BFS with optional call hierarchy.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn evaluate_impact_blast_radius(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };
        let depth = args.depth.unwrap_or(2);
        let include_depth_scores = args.include_depth_scores.unwrap_or(false);

        let symbol_name = args
            .symbol_id
            .as_deref()
            .or(args.target.as_deref())
            .or_else(|| {
                args.refs
                    .as_ref()
                    .and_then(|r| r.first())
                    .and_then(|r| r.stable_id.as_deref().or(r.id.as_deref()))
            })
            .unwrap_or("");

        let adj = store.dependency_graph()?;
        let mut reverse_adj: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (src, targets) in &adj {
            for tgt in targets {
                reverse_adj
                    .entry(tgt.clone())
                    .or_default()
                    .push(src.clone());
            }
        }

        let all_files = store.all_file_paths()?;
        let mut symbol_file: Option<String> = None;
        for file in &all_files {
            let outline = store.file_outline(file)?;
            for sym in &outline {
                if sym["name"].as_str() == Some(symbol_name) {
                    symbol_file = Some(file.clone());
                    break;
                }
            }
            if symbol_file.is_some() {
                break;
            }
        }

        let mut layers: Vec<(u64, Vec<String>)> = Vec::new();
        let mut visited = HashSet::new();
        let mut confirmed: Vec<Value> = Vec::new();
        let mut potential: Vec<Value> = Vec::new();

        if let Some(start_file) = &symbol_file {
            visited.insert(start_file.clone());
            let mut frontier = vec![start_file.clone()];

            for d in 1..=depth {
                let mut next_frontier = Vec::new();
                let mut layer_files = Vec::new();
                for f in &frontier {
                    if let Some(importers) = reverse_adj.get(f) {
                        for imp in importers {
                            if visited.insert(imp.clone()) {
                                next_frontier.push(imp.clone());
                                layer_files.push(imp.clone());
                                if d == 1 {
                                    confirmed.push(json!({
                                        "file": imp,
                                        "relation": "direct_importer",
                                        "depth": d,
                                    }));
                                } else {
                                    potential.push(json!({
                                        "file": imp,
                                        "relation": "transitive_importer",
                                        "depth": d,
                                    }));
                                }
                            }
                        }
                    }
                }
                if !layer_files.is_empty() {
                    layers.push((d, layer_files));
                }
                frontier = next_frontier;
                if frontier.is_empty() {
                    break;
                }
            }
        }

        let total_impacted = confirmed.len() + potential.len();
        let overall_risk = if total_impacted == 0 {
            0.0
        } else {
            (total_impacted as f64 / all_files.len().max(1) as f64).min(1.0)
        };
        let direct_count = confirmed.len();

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        let mut data = json!({
            "analysis": "blast_radius",
            "overall_risk_score": overall_risk,
            "direct_dependents_count": direct_count,
            "importer_count": total_impacted,
            "confirmed": confirmed,
            "potential": potential,
            "confirmed_count": confirmed.len(),
            "potential_count": potential.len(),
            "findings": layers.iter().map(|(d, files)| {
                json!({
                    "depth": d,
                    "files": files,
                    "risk_score": 1.0 / (*d as f64),
                })
            }).collect::<Vec<_>>(),
        });

        if include_depth_scores {
            let impact_by_depth: Vec<Value> = layers
                .iter()
                .map(|(d, files)| {
                    json!({
                        "depth": d,
                        "files": files,
                        "risk_score": 1.0 / (*d as f64),
                    })
                })
                .collect();
            data["impact_by_depth"] = json!(impact_by_depth);
        }

        let call_depth = args.call_depth.unwrap_or(0).min(3);
        if call_depth > 0 {
            let sym_infos = store.all_symbols_with_source()?;
            let target_sym = sym_infos.iter().find(|s| s.name == symbol_name);
            if let Some(target) = target_sym {
                let call_edges = crate::analysis::graph::build_call_edges(&sym_infos);
                let callers =
                    crate::analysis::graph::find_callers(&target.id, &call_edges, call_depth);
                let callers_json: Vec<Value> = callers
                    .iter()
                    .map(|(id, name, file, line, kind, depth)| {
                        json!({
                            "id": id, "name": name, "kind": kind,
                            "file": file, "line": line, "depth": depth,
                        })
                    })
                    .collect();
                data["callers"] = json!(callers_json);
                data["caller_count"] = json!(callers_json.len());
            }
        }

        let cross_repo = args.cross_repo.unwrap_or(self.config.cross_repo_default);
        if cross_repo {
            data["cross_repo_key"] = json!(format!("blast_radius:{}", symbol_name));
        }

        self.success_response(&timer, data)
    }
}
