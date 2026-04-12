mod gather;
mod shape;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn assemble_context_bundle(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let Some(refs) = args.refs.as_ref().filter(|r| !r.is_empty()) else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "context_bundle requires refs with a symbol ref.".to_string(),
                    detail: json!({}),
                },
            );
        };

        let sym_refs: Vec<_> = refs
            .iter()
            .filter(|r| r.kind.eq_ignore_ascii_case("symbol"))
            .collect();
        if sym_refs.is_empty() {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "context_bundle requires at least one symbol ref.".to_string(),
                    detail: json!({}),
                },
            );
        }

        let bs_raw = args.budget_strategy.as_deref().unwrap_or("").trim();
        if !bs_raw.is_empty() {
            let bsl = bs_raw.to_ascii_lowercase();
            if bsl != "compact" && bsl != "most_relevant" {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: format!("Unknown budget_strategy '{bs_raw}'."),
                        detail: json!({ "budget_strategy": bs_raw }),
                    },
                );
            }
        }
        let strategy_label = if bs_raw.is_empty() {
            None
        } else {
            Some(bs_raw.to_ascii_lowercase())
        };

        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };

        let rows = match self.gather_bundle_rows(&store, &sym_refs, timer)? {
            Ok(rows) => rows,
            Err(early) => return Ok(early),
        };

        let budget = shape::apply_budget(rows, args.max_tokens, &strategy_label);
        self.shape_bundle_response(budget, &sym_refs, &store, &strategy_label, args)
    }
}
