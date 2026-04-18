//! `evaluate_impact analysis=blast_radius` — reverse-import BFS with optional call hierarchy.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::IndexStore;

/// Build reverse adjacency (file → its importers) from forward dep graph.
fn build_reverse_adj(adj: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<String>> {
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
    for (src, targets) in adj {
        for tgt in targets {
            reverse.entry(tgt.clone()).or_default().push(src.clone());
        }
    }
    reverse
}

/// Resolve the file that contains the target symbol via a single indexed
/// `WHERE name = ?` lookup against the `symbols` table.
fn resolve_symbol_file(
    store: &IndexStore,
    stable_id: &str,
    symbol_name: &str,
) -> anyhow::Result<Option<String>> {
    if !stable_id.is_empty()
        && let Ok(Some(rec)) = store.symbol_record(stable_id)
    {
        return Ok(Some(rec.file_path));
    }
    if !symbol_name.is_empty() {
        return store.file_for_symbol_name(symbol_name);
    }
    Ok(None)
}

struct BfsResult {
    confirmed: Vec<Value>,
    potential: Vec<Value>,
    layers: Vec<(u64, Vec<String>)>,
}

/// BFS over reverse-import edges from `start_file`, up to `depth` layers.
fn bfs_importers(
    start_file: &str,
    reverse_adj: &HashMap<String, Vec<String>>,
    depth: u64,
) -> BfsResult {
    let mut layers = Vec::new();
    let mut visited = HashSet::new();
    let mut confirmed = Vec::new();
    let mut potential = Vec::new();

    visited.insert(start_file.to_string());
    let mut frontier = vec![start_file.to_string()];

    for d in 1..=depth {
        let mut next_frontier = Vec::new();
        let mut layer_files = Vec::new();
        for f in &frontier {
            if let Some(importers) = reverse_adj.get(f) {
                for imp in importers {
                    if visited.insert(imp.clone()) {
                        next_frontier.push(imp.clone());
                        layer_files.push(imp.clone());
                        let entry = json!({
                            "file": imp,
                            "relation": if d == 1 { "direct_importer" } else { "transitive_importer" },
                            "depth": d,
                        });
                        if d == 1 {
                            &mut confirmed
                        } else {
                            &mut potential
                        }
                        .push(entry);
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
    BfsResult {
        confirmed,
        potential,
        layers,
    }
}

impl CodeService {
    pub(super) fn evaluate_impact_blast_radius(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);
        let depth = args.depth.unwrap_or(2);

        let stable_id = args
            .symbol_id
            .as_deref()
            .or_else(|| {
                args.refs
                    .as_ref()
                    .and_then(|r| r.first())
                    .and_then(|r| r.stable_id.as_deref().or(r.id.as_deref()))
            })
            .unwrap_or("");
        let symbol_name = args.target.as_deref().unwrap_or_else(|| {
            stable_id
                .rsplit("::")
                .next()
                .and_then(|tail| tail.split('#').next())
                .unwrap_or("")
        });

        let adj = store.dependency_graph()?;
        let reverse_adj = build_reverse_adj(&adj);
        let all_files = store.all_file_paths()?;
        let symbol_file = resolve_symbol_file(&store, stable_id, symbol_name)?;

        let bfs = symbol_file
            .as_deref()
            .map(|f| bfs_importers(f, &reverse_adj, depth))
            .unwrap_or_else(|| BfsResult {
                confirmed: Vec::new(),
                potential: Vec::new(),
                layers: Vec::new(),
            });

        let total_impacted = bfs.confirmed.len() + bfs.potential.len();
        let overall_risk = if total_impacted == 0 {
            0.0
        } else {
            (total_impacted as f64 / all_files.len().max(1) as f64).min(1.0)
        };

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        let layer_findings: Vec<Value> = bfs
            .layers
            .iter()
            .map(
                |(d, files)| json!({ "depth": d, "files": files, "risk_score": 1.0 / (*d as f64) }),
            )
            .collect();

        let mut data = json!({
            "analysis": "blast_radius",
            "overall_risk_score": overall_risk,
            "direct_dependents_count": bfs.confirmed.len(),
            "importer_count": total_impacted,
            "confirmed": bfs.confirmed,
            "potential": bfs.potential,
            "confirmed_count": bfs.confirmed.len(),
            "potential_count": bfs.potential.len(),
            "findings": layer_findings,
        });

        if args.include_depth_scores.unwrap_or(false) {
            data["impact_by_depth"] = json!(layer_findings);
        }

        let call_depth = args.call_depth.unwrap_or(0).min(3);
        if call_depth > 0 {
            let sym_infos = store.all_symbols_with_source()?;
            if let Some(target) = sym_infos.iter().find(|s| s.name == symbol_name) {
                let call_edges = codesight_analysis::graph::build_call_edges(&sym_infos);
                let callers =
                    codesight_analysis::graph::find_callers(&target.id, &call_edges, call_depth);
                let callers_json: Vec<Value> = callers
                    .iter()
                    .map(|(id, name, file, line, kind, depth)| {
                        json!({
                            "id": id, "name": name, "kind": kind,
                            "file": file, "line": line, "depth": depth,
                        })
                    })
                    .collect();
                data["caller_count"] = json!(callers_json.len());
                data["callers"] = json!(callers_json);
            }
        }

        if args.cross_repo.unwrap_or(false) {
            data["cross_repo_key"] = json!(format!("blast_radius:{}", symbol_name));
        }

        self.success_response(&timer, data)
    }
}
