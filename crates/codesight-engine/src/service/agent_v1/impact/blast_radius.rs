//! `evaluate_impact analysis=blast_radius` — reverse-import BFS with optional call hierarchy.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{ContentRef, EvaluateImpactArgs};
use crate::service::helpers::{ToolError, ToolErrorCategory};
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

fn looks_like_stable_symbol_id(id: &str) -> bool {
    id.contains("::") && id.contains('#')
}

fn ref_requires_strict_resolution(r: &ContentRef) -> bool {
    r.path.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
        || r.start_line.is_some()
        || r.symbol_name
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        || r.id
            .as_deref()
            .map(looks_like_stable_symbol_id)
            .unwrap_or(false)
        || r.stable_id
            .as_deref()
            .map(looks_like_stable_symbol_id)
            .unwrap_or(false)
}

fn unresolved_ref_error(r: &ContentRef) -> ToolError {
    ToolError {
        category: ToolErrorCategory::NotFound,
        code: "NOT_FOUND",
        message: "blast_radius could not resolve refs[0].".to_string(),
        detail: json!({ "unresolved": [r] }),
    }
}

struct BfsResult {
    /// FIXUP T2.4: flat file lists, no per-row objects — callers don't need
    /// `relation` (implied by bucket) or `depth` (implied by bucket / 1-vs-N).
    direct_importers: Vec<String>,
    transitive_importers: Vec<(String, u64)>,
    layers: Vec<(u64, Vec<String>)>,
}

/// BFS over reverse-import edges from `start_file`, up to `depth` layers.
/// When `include_vendored` is false, importer nodes whose paths match the
/// default vendor exclusions are dropped from `direct` / `transitive` /
/// `layers` (they're still traversed internally so the BFS doesn't stall
/// on a legitimate importer sitting behind a vendored edge).
fn bfs_importers(
    start_file: &str,
    reverse_adj: &HashMap<String, Vec<String>>,
    depth: u64,
    include_vendored: bool,
) -> BfsResult {
    let mut layers = Vec::new();
    let mut visited = HashSet::new();
    let mut direct_importers = Vec::new();
    let mut transitive_importers: Vec<(String, u64)> = Vec::new();

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
                        let excluded = !include_vendored
                            && codesight_language_support::is_vendored_path(imp);
                        if excluded {
                            continue;
                        }
                        layer_files.push(imp.clone());
                        if d == 1 {
                            direct_importers.push(imp.clone());
                        } else {
                            transitive_importers.push((imp.clone(), d));
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
    BfsResult {
        direct_importers,
        transitive_importers,
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

        // Explicit-import edges alone miss intra-crate / same-package
        // references (Rust `mod x; x::Y`, Python sibling access, Go
        // same-package). Union with symbol-usage edges so the BFS
        // frontier matches what `find_references` would see.
        let adj = codesight_analysis::graph::adjacency_with_usage_edges(&store)?;
        let reverse_adj = build_reverse_adj(&adj);
        let all_files = store.all_file_paths()?;
        let first_ref = args.refs.as_ref().and_then(|r| r.first());
        let mut resolution: Option<&'static str> = None;
        let symbol_file = if let Some(r) = first_ref
            .filter(|r| r.kind.eq_ignore_ascii_case("symbol") && ref_requires_strict_resolution(r))
        {
            // ROADMAP P0.2 #3: promote to strict resolution so we can echo
            // which ref shape actually resolved and emit NOT_FOUND when none
            // do. Previously used the permissive `resolve_symbol_id`.
            use crate::service::agent_v1::util::resolve_symbol_id_strict;
            let resolved = resolve_symbol_id_strict(
                &store,
                r.stable_id.as_deref(),
                r.id.as_deref(),
                r.path.as_deref(),
                r.start_line,
                r.symbol_name.as_deref(),
            )?;
            match resolved {
                Some((id, strategy)) => {
                    resolution = Some(strategy.as_str());
                    match store.symbol_record(&id)?.map(|rec| rec.file_path) {
                        Some(file) => Some(file),
                        None => return self.error_response(&timer, unresolved_ref_error(r)),
                    }
                }
                None => return self.error_response(&timer, unresolved_ref_error(r)),
            }
        } else {
            resolve_symbol_file(&store, stable_id, symbol_name)?
        };

        let include_vendored = args.include_vendored.unwrap_or(false)
            || args
                .scope
                .as_deref()
                .map(codesight_language_support::scope_opts_in)
                .unwrap_or(false);
        let bfs = symbol_file
            .as_deref()
            .map(|f| bfs_importers(f, &reverse_adj, depth, include_vendored))
            .unwrap_or_else(|| BfsResult {
                direct_importers: Vec::new(),
                transitive_importers: Vec::new(),
                layers: Vec::new(),
            });

        let direct_count = bfs.direct_importers.len();
        let transitive_count = bfs.transitive_importers.len();
        let total_impacted = direct_count + transitive_count;
        let overall_risk = if total_impacted == 0 {
            0.0
        } else {
            (total_impacted as f64 / all_files.len().max(1) as f64).min(1.0)
        };

        // ROADMAP P1.1: impact analysis is structural — no native-tool
        // counterfactual. Null baseline (`tokens_saved: null`).
        let timer = MetaBuilder::new(0);

        // FIXUP T2.4: `findings` is a depth-histogram; the file list is
        // already surfaced via `direct_importers` / `transitive_importers`.
        // Dropping `files` from `findings` eliminates the ~3× payload
        // duplication the plan flagged.
        let layer_findings: Vec<Value> = bfs
            .layers
            .iter()
            .map(|(d, files)| {
                json!({
                    "depth": d,
                    "count": files.len(),
                    "risk_score": 1.0 / (*d as f64),
                })
            })
            .collect();

        // Keep the detailed per-depth view (with file lists) available
        // under the opt-in `impact_by_depth` field so callers who need it
        // aren't forced to re-derive it.
        let impact_by_depth: Vec<Value> = bfs
            .layers
            .iter()
            .map(|(d, files)| {
                json!({
                    "depth": d,
                    "files": files,
                    "risk_score": 1.0 / (*d as f64),
                })
            })
            .collect();

        let transitive_json: Vec<Value> = bfs
            .transitive_importers
            .iter()
            .map(|(file, depth)| json!({ "file": file, "depth": depth }))
            .collect();

        let mut data = json!({
            "analysis": "blast_radius",
            "overall_risk_score": overall_risk,
            "importer_count": total_impacted,
            "direct_importers": bfs.direct_importers,
            "transitive_importers": transitive_json,
            "direct_count": direct_count,
            "transitive_count": transitive_count,
            "findings": layer_findings,
        });

        if args.include_depth_scores.unwrap_or(false) {
            data["impact_by_depth"] = json!(impact_by_depth);
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

        // ROADMAP P2.8: emit actionable_next hints for the top direct
        // importers so the agent can inspect callers or rename-check each
        // one without replaying the blast-radius path.
        // FIXUP T2.4: iterate over the flat Vec<String> direct_importers —
        // previously this ran `.as_str()` on {file, relation, depth}
        // objects (always None), so actionable_next was silently empty.
        let actionable_next: Vec<Value> = bfs
            .direct_importers
            .iter()
            .take(3)
            .map(|path| {
                json!({
                    "tool": "inspect_graph",
                    "args": {
                        "source_id": args.source_id,
                        "graph": "references",
                        "ref": { "path": path },
                    },
                    "reason": format!("Enumerate references into `{path}` before editing the target."),
                })
            })
            .collect();

        // ROADMAP P0.2 #3: echo ref-shape resolution + P2.8 actionable_next.
        let mut extras: Vec<(String, Value)> = vec![(
            "actionable_next".to_string(),
            json!(actionable_next),
        )];
        if let Some(r) = resolution {
            extras.push(("resolution".to_string(), json!(r)));
        }
        self.success_response_with_meta_extra(&timer, data, Vec::new(), extras)
    }
}
