//! Impact analyses (diff, blast radius, …).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util_graph::{
    extract_symbol_names_from_content, git_head_sha, git_show_file, simple_text_diff,
};
use crate::service::args::parse_args;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn evaluate_impact(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateImpactArgs = parse_args(v)?;
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        if a == "diff" {
            let Some(refs) = args.refs.as_ref() else {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: "diff requires refs array.".to_string(),
                        detail: json!({}),
                    },
                );
            };
            if refs.len() < 2 {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: "diff requires two symbol refs.".to_string(),
                        detail: json!({}),
                    },
                );
            }
            let id_a = refs[0]
                .stable_id
                .clone()
                .or(refs[0].id.clone())
                .filter(|_| refs[0].kind.eq_ignore_ascii_case("symbol"));
            let id_b = refs[1]
                .stable_id
                .clone()
                .or(refs[1].id.clone())
                .filter(|_| refs[1].kind.eq_ignore_ascii_case("symbol"));
            let (Some(id_a), Some(id_b)) = (id_a, id_b) else {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: "diff requires two symbol refs with kind symbol.".to_string(),
                        detail: json!({}),
                    },
                );
            };
            let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => s,
                Err(e) => return self.error_response(&timer, e),
            };
            let ra = store.symbol_record(&id_a)?;
            let rb = store.symbol_record(&id_b)?;
            let (Some(ra), Some(rb)) = (ra, rb) else {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "NOT_FOUND",
                        message: "One or both symbols missing from index.".to_string(),
                        detail: json!({ "id_a": id_a, "id_b": id_b }),
                    },
                );
            };
            if ra.file_path != rb.file_path {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: "diff is supported for symbols in the same file only.".to_string(),
                        detail: json!({ "file_a": ra.file_path, "file_b": rb.file_path }),
                    },
                );
            }
            let raw_bytes = ra.file_size_bytes.max(rb.file_size_bytes);
            let timer = MetaBuilder::new(raw_bytes);
            let patch = simple_text_diff(&ra.stored_source, &rb.stored_source);
            return self.success_response(
                &timer,
                json!({
                    "analysis": "diff",
                    "findings": [{
                        "finding_id": "diff:symbol_pair",
                        "analysis": "diff",
                        "severity": "info",
                        "confidence": 1.0,
                        "summary": "line-oriented text diff of indexed symbol spans",
                        "evidence": patch,
                        "related_refs": [id_a, id_b],
                    }],
                }),
            );
        }

        if a == "blast_radius" {
            return self.evaluate_impact_blast_radius(&args);
        }
        if a == "changed_units" {
            return self.evaluate_impact_changed_units(&args);
        }
        if a == "churn_rate" {
            return self.evaluate_impact_churn_rate(&args);
        }
        if a == "impact_preview" {
            return self.evaluate_impact_impact_preview(&args);
        }
        if a == "rename_safety" {
            return self.evaluate_impact_rename_safety(&args);
        }

        let warnings = vec![json!({
            "code": "NOT_IMPLEMENTED",
            "message": format!("impact analysis '{a}' is not implemented."),
        })];
        self.success_response_with_warnings(
            &timer,
            json!({ "analysis": a, "findings": [] }),
            warnings,
        )
    }

    // ── blast_radius ────────────────────────────────────────────────────────
    fn evaluate_impact_blast_radius(&self, args: &EvaluateImpactArgs) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };
        let depth = args.depth.unwrap_or(2);
        let include_depth_scores = args.include_depth_scores.unwrap_or(false);

        // Extract symbol name from symbol_id, target, or refs
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

        // Build reverse dependency graph (who imports each file)
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

        // Find which file contains the symbol
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

        // BFS from the file containing the symbol
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

        // call_depth: if > 0, add callers via call hierarchy
        let call_depth = args.call_depth.unwrap_or(0).min(3);
        if call_depth > 0 {
            let sym_infos = store.all_symbols_with_source()?;
            // Find the symbol id by name
            let target_sym = sym_infos.iter().find(|s| s.name == symbol_name);
            if let Some(target) = target_sym {
                let call_edges = crate::dep_graph::build_call_edges(&sym_infos);
                let callers = crate::dep_graph::find_callers(&target.id, &call_edges, call_depth);
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

        // cross_repo support
        let cross_repo = args.cross_repo.unwrap_or(self.config.cross_repo_default);
        if cross_repo {
            data["cross_repo_key"] = json!(format!("blast_radius:{}", symbol_name));
        }

        self.success_response(&timer, data)
    }

    // ── changed_units ───────────────────────────────────────────────────────
    fn evaluate_impact_changed_units(&self, args: &EvaluateImpactArgs) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };

        let root = match store.root_path() {
            Ok(r) => r,
            Err(_) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "NOT_LOCAL",
                        message: "Source is not a local repository.".to_string(),
                        detail: json!({}),
                    },
                );
            }
        };

        if !root.is_dir() {
            return self.error_response(
                &timer,
                ToolError {
                    code: "NOT_LOCAL",
                    message: "Source root is not accessible.".to_string(),
                    detail: json!({}),
                },
            );
        }

        // Extract commit refs
        let commit_refs: Vec<String> = args
            .refs
            .as_ref()
            .map(|refs| {
                refs.iter()
                    .filter(|r| r.kind.eq_ignore_ascii_case("commit"))
                    .filter_map(|r| r.id.clone().or(r.stable_id.clone()))
                    .collect()
            })
            .unwrap_or_default();

        // Determine the two SHAs to compare
        let (since_sha, until_sha) = if commit_refs.len() >= 2 {
            (commit_refs[0].clone(), commit_refs[1].clone())
        } else if commit_refs.len() == 1 {
            // Compare since_sha to HEAD
            let head = git_head_sha(&root);
            match head {
                Some(h) => (commit_refs[0].clone(), h),
                None => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "GIT_ERROR",
                            message: "Cannot determine HEAD SHA.".to_string(),
                            detail: json!({}),
                        },
                    );
                }
            }
        } else {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: "changed_units requires at least one commit ref (since_sha)."
                        .to_string(),
                    detail: json!({}),
                },
            );
        };

        // Get the list of changed files between the two commits
        let diff_output = std::process::Command::new("git")
            .args(["diff", "--name-only", &since_sha, &until_sha])
            .current_dir(&root)
            .output();

        let diff_output = match diff_output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "GIT_ERROR",
                        message: format!("git diff failed between {since_sha} and {until_sha}."),
                        detail: json!({ "since": since_sha, "until": until_sha }),
                    },
                );
            }
        };

        let changed_files: Vec<&str> = diff_output.lines().filter(|l| !l.is_empty()).collect();
        let extractors = self.extractors_for_root(&root);

        // For each changed file, extract symbols at both SHAs and compare
        let mut added_symbols = Vec::new();
        let mut removed_symbols = Vec::new();
        let mut changed_symbols = Vec::new();

        for file in &changed_files {
            let old_content = git_show_file(&root, &since_sha, file);
            let new_content = git_show_file(&root, &until_sha, file);

            let old_syms = extract_symbol_names_from_content(
                old_content.as_deref().unwrap_or(""),
                file,
                &extractors,
            );
            let new_syms = extract_symbol_names_from_content(
                new_content.as_deref().unwrap_or(""),
                file,
                &extractors,
            );

            // Build maps: name -> source text
            let old_map: std::collections::HashMap<&str, &str> = old_syms
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();
            let new_map: std::collections::HashMap<&str, &str> = new_syms
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();

            for (name, _src) in &new_syms {
                if !old_map.contains_key(name.as_str()) {
                    added_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "added",
                        "blast_radius": 0,
                    }));
                } else if old_map.get(name.as_str()) != Some(&_src.as_str()) {
                    changed_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "modified",
                        "blast_radius": 0,
                    }));
                }
            }
            for (name, _) in &old_syms {
                if !new_map.contains_key(name.as_str()) {
                    removed_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "removed",
                        "blast_radius": 0,
                    }));
                }
            }
        }

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        self.success_response(
            &timer,
            json!({
                "analysis": "changed_units",
                "added_symbols": added_symbols,
                "removed_symbols": removed_symbols,
                "changed_symbols": changed_symbols,
                "added_count": added_symbols.len(),
                "removed_count": removed_symbols.len(),
                "changed_count": changed_symbols.len(),
            }),
        )
    }
}
