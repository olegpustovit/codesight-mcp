//! `inspect_graph graph=references|related` — symbol-scoped reference and
//! relatedness handlers.

use codesight_retrieval::fuzzy;
use codesight_storage::storage::SearchSymbol;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::agent_v1::util::graph::graph_ref_symbol_id;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

impl CodeService {
    pub(super) fn graph_references(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        limit: usize,
    ) -> anyhow::Result<Value> {
        let sym_id = match graph_ref_symbol_id(args.r#ref.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: e.to_string(),
                        detail: json!({}),
                    },
                );
            }
        };
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        // ROADMAP P1.1: reference/related graphs are structural analysis.
        // Null baseline — there's no native-tool counterfactual sized like
        // a whole-repo read.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Heuristic);
        let needle = match store.symbol_names_by_id(&sym_id)? {
            Some((name, qn)) => {
                if name.len() >= 3 {
                    name
                } else {
                    qn
                }
            }
            None => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found."),
                        detail: json!({ "id": sym_id }),
                    },
                );
            }
        };
        let hits = store.symbols_source_substring(&needle, &sym_id, limit)?;
        let mut edges = Vec::new();
        for h in hits {
            edges.push(json!({
                "relation": "references",
                "from_ref": { "stable_id": sym_id, "source_id": args.source_id, "kind": "symbol" },
                "to_ref": {
                    "stable_id": h.get("id"),
                    "kind": h.get("kind"),
                    "path": h.get("file"),
                    "start_line": h.get("line"),
                },
            }));
        }
        self.success_response_with_warnings(
            &timer,
            json!({
                "graph": mode,
                "methodology": "substring_match_in_symbol_spans",
                "edges": edges,
                "nodes": [],
            }),
            vec![json!({
                "code": "HEURISTIC_GRAPH",
                "message": "references match inside indexed symbol bodies only; not IDE-accurate.",
            })],
        )
    }

    pub(super) fn graph_related(
        &self,
        args: &InspectGraphArgs,
        mode: &str,
        limit: usize,
    ) -> anyhow::Result<Value> {
        let sym_id = match graph_ref_symbol_id(args.r#ref.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: e.to_string(),
                        detail: json!({}),
                    },
                );
            }
        };
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let Some((sym_name, _qn)) = store.symbol_names_by_id(&sym_id)? else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Symbol '{sym_id}' not found."),
                    detail: json!({ "id": sym_id }),
                },
            );
        };
        let record = match store.symbol_record(&sym_id)? {
            Some(r) => r,
            None => {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found."),
                        detail: json!({ "id": sym_id }),
                    },
                );
            }
        };
        // ROADMAP P1.1: reference/related graphs are structural analysis.
        // Null baseline — there's no native-tool counterfactual sized like
        // a whole-repo read.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Heuristic);
        let scan_cap = limit.saturating_mul(400).clamp(2_000, 12_000);
        let candidates = match store.symbols_light_scan(None, None, scan_cap) {
            Ok(c) => c,
            Err(e) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::Internal,
                        code: "INDEX_ERROR",
                        message: e.to_string(),
                        detail: json!({}),
                    },
                );
            }
        };
        let mut scored: Vec<SearchSymbol> = Vec::new();
        for mut o in candidates {
            if o.id == sym_id {
                continue;
            }
            let rel = fuzzy::relatedness_score(&sym_name, &record.file_path, &o.name, &o.file);
            if rel > 0.0 {
                o.score = rel;
                scored.push(o);
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        scored.truncate(limit);
        let target = json!({
            "id": record.id,
            "name": sym_name,
            "kind": record.kind.as_str(),
            "file": record.file_path,
            "line": record.start_line,
        });
        let related: Vec<Value> = scored
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.id,
                    "name": s.name,
                    "kind": s.kind.as_str(),
                    "file": s.file,
                    "line": s.line,
                    "relatedness_score": s.score,
                })
            })
            .collect();
        self.success_response(
            &timer,
            json!({
                "graph": mode,
                "symbol": target,
                "related": related,
                "related_count": related.len(),
            }),
        )
    }
}
