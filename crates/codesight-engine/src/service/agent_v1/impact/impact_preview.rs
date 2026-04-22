//! `evaluate_impact analysis=impact_preview` — BFS over call edges.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util::graph::{build_caller_parents, reconstruct_caller_chain};
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_impact_preview(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);

        let sym_name_from_id = args.symbol_id.as_deref().unwrap_or("");
        let sym_id_from_refs = args
            .refs
            .as_ref()
            .and_then(|r| r.first())
            .and_then(|r| r.stable_id.as_deref().or(r.id.as_deref()));

        let (sym_id, sym_name) = if !sym_name_from_id.is_empty() {
            let sym_infos = store.all_symbols_with_source()?;
            let found = sym_infos.iter().find(|s| s.name == sym_name_from_id);
            match found {
                Some(s) => (s.id.clone(), s.name.clone()),
                None => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::NotFound,
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
                            category: ToolErrorCategory::NotFound,
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
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message:
                        "impact_preview requires symbol_id or refs[0] with a symbol stable_id."
                            .to_string(),
                    detail: json!({}),
                },
            );
        };

        let sym_infos = store.all_symbols_with_source()?;
        let edges = codesight_analysis::graph::build_call_edges(&sym_infos);
        let callers = codesight_analysis::graph::find_callers(&sym_id, &edges, 3);

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

        let caller_parents = build_caller_parents(&sym_id, &edges);
        let mut call_chains: Vec<Value> = Vec::new();
        for (cid, _cname, _cfile, _cline, _ckind, _depth) in &callers {
            let chain = reconstruct_caller_chain(&sym_id, cid, &caller_parents);
            call_chains.push(json!({
                "symbol_id": cid,
                "chain": chain,
            }));
        }

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
}
