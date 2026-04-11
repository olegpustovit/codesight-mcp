//! `inspect_graph graph=calls` — text-heuristic call hierarchy over indexed symbols.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn graph_calls(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };
        let Some(symbol_id) = args.symbol_id.as_deref() else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "calls mode requires symbol_id.".into(),
                    detail: json!({}),
                },
            );
        };

        let sym_infos = store.all_symbols_with_source()?;

        let Some(target) = sym_infos.iter().find(|s| s.id == symbol_id) else {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Symbol '{symbol_id}' not found in index."),
                    detail: json!({ "symbol_id": symbol_id }),
                },
            );
        };

        let edges = crate::analysis::graph::build_call_edges(&sym_infos);
        let direction = args.direction.as_deref().unwrap_or("both");
        let depth = args.depth.unwrap_or(3);

        let caller_tuples = if direction == "callees" {
            vec![]
        } else {
            crate::analysis::graph::find_callers(symbol_id, &edges, depth)
        };
        let callee_tuples = if direction == "callers" {
            vec![]
        } else {
            crate::analysis::graph::find_callees(symbol_id, &edges, depth)
        };

        let to_json = |tuples: &[(String, String, String, u32, String, usize)]| -> Vec<Value> {
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
}
