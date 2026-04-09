//! Impact analyses (churn, preview, rename safety).

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::analysis::impact;
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util_graph::build_call_chain;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_churn_rate(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };

        let target = args.target.as_deref().unwrap_or("");
        let days = args.days.unwrap_or(90);

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

        // Determine if target is a symbol ID or file path
        let is_symbol = target.contains("::")
            || target.contains('#')
            || store.symbol_record(target).ok().flatten().is_some();
        let (target_type, file_path, symbol_name) = if is_symbol {
            // Look up symbol to get its file
            let rec = store.symbol_record(target)?;
            match rec {
                Some(r) => {
                    let name =
                        r.id.rsplit("::")
                            .next()
                            .unwrap_or("")
                            .split('#')
                            .next()
                            .unwrap_or("")
                            .to_string();
                    ("symbol".to_string(), r.file_path.clone(), Some(name))
                }
                None => {
                    // Try to find by name
                    let results = store.search_symbols_name_contains(target, None, None, 1)?;
                    if let Some(sym) = results.first() {
                        (
                            "symbol".to_string(),
                            sym.file.clone(),
                            Some(sym.name.clone()),
                        )
                    } else {
                        return self.error_response(
                            &timer,
                            ToolError {
                                code: "NOT_FOUND",
                                message: format!("Target '{target}' not found."),
                                detail: json!({ "target": target }),
                            },
                        );
                    }
                }
            }
        } else {
            ("file".to_string(), target.to_string(), None)
        };

        // Try git log
        let git_result = std::process::Command::new("git")
            .args([
                "log",
                "--oneline",
                &format!("--since={days} days ago"),
                "--follow",
                "--",
                &file_path,
            ])
            .current_dir(&root)
            .output();

        let (commits, authors, churn_per_week) = match git_result {
            Ok(o) if o.status.success() => {
                let log = String::from_utf8_lossy(&o.stdout);
                let commit_count = log.lines().filter(|l| !l.is_empty()).count();

                // Get unique authors
                let author_output = std::process::Command::new("git")
                    .args([
                        "log",
                        &format!("--since={days} days ago"),
                        "--format=%an",
                        "--follow",
                        "--",
                        &file_path,
                    ])
                    .current_dir(&root)
                    .output()
                    .ok();
                let authors = author_output
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .filter(|l| !l.is_empty())
                            .collect::<HashSet<_>>()
                            .len()
                    })
                    .unwrap_or(0);

                let weeks = (days as f64 / 7.0).max(1.0);
                let cpw = commit_count as f64 / weeks;
                (commit_count, authors, cpw)
            }
            _ => {
                // No git available - return heuristic stub
                (0, 0, 0.0)
            }
        };

        let assessment = if churn_per_week < 0.5 {
            "stable"
        } else if churn_per_week < 2.0 {
            "active"
        } else {
            "volatile"
        };

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        let mut data = json!({
            "analysis": "churn_rate",
            "target_type": target_type,
            "file": file_path,
            "commits": commits,
            "authors": authors,
            "churn_per_week": churn_per_week,
            "assessment": assessment,
            "days": days,
        });
        if let Some(sname) = &symbol_name {
            data["symbol_name"] = json!(sname);
        }

        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            vec![
                ("methodology".to_string(), json!("git_log")),
                ("confidence_level".to_string(), json!("high")),
            ],
        )
    }

    // ── impact_preview ──────────────────────────────────────────────────────
    pub(in crate::service) fn evaluate_impact_impact_preview(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };

        // Accept symbol_id directly OR from refs[0]
        let sym_name_from_id = args.symbol_id.as_deref().unwrap_or("");
        let sym_id_from_refs = args
            .refs
            .as_ref()
            .and_then(|r| r.first())
            .and_then(|r| r.stable_id.as_deref().or(r.id.as_deref()));

        // Resolve symbol: either by name lookup or by stable_id
        let (sym_id, sym_name) = if !sym_name_from_id.is_empty() {
            // symbol_id param is a name; look it up
            let sym_infos = store.all_symbols_with_source()?;
            let found = sym_infos.iter().find(|s| s.name == sym_name_from_id);
            match found {
                Some(s) => (s.id.clone(), s.name.clone()),
                None => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!("Symbol '{}' not found in index.", sym_name_from_id),
                            detail: json!({ "symbol_id": sym_name_from_id }),
                        },
                    );
                }
            }
        } else if let Some(sid) = sym_id_from_refs {
            let names = store.symbol_names_by_id(sid)?;
            match names {
                Some((n, _)) => (sid.to_string(), n),
                None => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!("Symbol '{}' not found in index.", sid),
                            detail: json!({ "stable_id": sid }),
                        },
                    );
                }
            }
        } else {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message:
                        "impact_preview requires symbol_id or refs[0] with a symbol stable_id."
                            .to_string(),
                    detail: json!({}),
                },
            );
        };

        // Find callers using call edges
        let sym_infos = store.all_symbols_with_source()?;
        let edges = crate::dep_graph::build_call_edges(&sym_infos);
        let callers = crate::dep_graph::find_callers(&sym_id, &edges, 3);

        // Build affected_symbols (deduplicated)
        let mut affected_symbols: Vec<Value> = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut affected_by_file: std::collections::HashMap<String, Vec<Value>> =
            std::collections::HashMap::new();

        for (cid, cname, cfile, cline, ckind, _depth) in &callers {
            if seen_ids.insert(cid.clone()) {
                let sym_val = json!({
                    "id": cid,
                    "name": cname,
                    "file": cfile,
                    "line": cline,
                    "kind": ckind,
                });
                affected_symbols.push(sym_val.clone());
                affected_by_file
                    .entry(cfile.clone())
                    .or_default()
                    .push(sym_val);
            }
        }

        let affected_files_count = affected_by_file.len();

        // Build call chains: for each caller, the chain from target -> ... -> caller
        let mut call_chains: Vec<Value> = Vec::new();
        for (cid, _cname, _cfile, _cline, _ckind, _depth) in &callers {
            // Build chain: trace path from sym_id to cid
            let chain = build_call_chain(&sym_id, cid, &edges);
            call_chains.push(json!({
                "symbol_id": cid,
                "chain": chain,
            }));
        }

        // Find the target symbol info
        let target_info = sym_infos.iter().find(|s| s.id == sym_id);
        let symbol_json = if let Some(t) = target_info {
            json!({
                "id": t.id,
                "name": t.name,
                "kind": t.kind,
                "file": t.file,
                "line": t.line,
            })
        } else {
            json!({ "id": sym_id, "name": sym_name })
        };

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        self.success_response_with_meta_extra(
            &timer,
            json!({
                "analysis": "impact_preview",
                "symbol": symbol_json,
                "affected_files": affected_files_count,
                "affected_symbol_count": affected_symbols.len(),
                "affected_symbols": affected_symbols,
                "affected_by_file": affected_by_file,
                "call_chains": call_chains,
                "findings": affected_symbols.iter().map(|s| {
                    json!({
                        "finding_id": format!("impact_preview:{}", s["id"].as_str().unwrap_or("")),
                        "analysis": "impact_preview",
                        "severity": "info",
                        "confidence": 0.3,
                        "methodology": "text_heuristic",
                        "summary": format!(
                            "{} in {} may reference {}",
                            s["name"].as_str().unwrap_or("?"),
                            s["file"].as_str().unwrap_or("?"),
                            sym_name
                        ),
                        "related_refs": [s["id"]],
                    })
                }).collect::<Vec<_>>(),
            }),
            Vec::new(),
            [("source".to_string(), json!("text_heuristic"))],
        )
    }

    // ── rename_safety ───────────────────────────────────────────────────────
    pub(in crate::service) fn evaluate_impact_rename_safety(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&timer, e),
        };

        let r = args.refs.as_ref().and_then(|refs| refs.first());

        let Some(r) = r else {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires refs[0].".to_string(),
                    detail: json!({}),
                },
            );
        };

        let sym_id = r.stable_id.as_deref().or(r.id.as_deref()).unwrap_or("");

        let new_name = r.new_name.as_deref().unwrap_or("");

        if new_name.is_empty() {
            return self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires new_name on the ref.".to_string(),
                    detail: json!({}),
                },
            );
        }
        let data = match impact::rename_safety_payload(&store, sym_id, new_name)? {
            Some(data) => data,
            None => {
                return self.error_response(
                    &timer,
                    ToolError {
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found in index."),
                        detail: json!({ "stable_id": sym_id }),
                    },
                );
            }
        };
        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);
        self.success_response(&timer, data)
    }
}
