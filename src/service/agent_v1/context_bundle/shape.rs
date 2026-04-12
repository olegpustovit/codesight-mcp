use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{AssembleContextArgs, ContentRef};
use crate::service::agent_v1::util::{resolve_symbol_id, token_estimate_for_context};
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use crate::storage::IndexStore;

use super::gather::BundleRow;

pub(super) struct BudgetResult {
    pub rows: Vec<BundleRow>,
    pub included_symbols: Vec<String>,
    pub excluded_symbols: Vec<String>,
}

pub(super) fn apply_budget(
    rows: Vec<BundleRow>,
    max_tokens: Option<u32>,
    strategy_label: &Option<String>,
) -> BudgetResult {
    let mut included_symbols = Vec::new();
    let mut excluded_symbols = Vec::new();

    let mut rows = if let Some(bt) = max_tokens {
        let mut kept = Vec::new();
        let mut used: u64 = 0;
        for row in rows {
            let t = token_estimate_for_context(&row.source) as u64;
            if used.saturating_add(t) > bt as u64 {
                excluded_symbols.push(row.sym_id.clone());
            } else {
                used = used.saturating_add(t);
                included_symbols.push(row.sym_id.clone());
                kept.push(row);
            }
        }
        kept
    } else {
        for row in &rows {
            included_symbols.push(row.sym_id.clone());
        }
        rows
    };

    if strategy_label.as_deref() == Some("compact") {
        for r in &mut rows {
            r.source.clear();
        }
    }

    BudgetResult {
        rows,
        included_symbols,
        excluded_symbols,
    }
}

impl CodeService {
    pub(super) fn shape_bundle_response(
        &self,
        budget: BudgetResult,
        sym_refs: &[&ContentRef],
        store: &IndexStore,
        strategy_label: &Option<String>,
        args: &AssembleContextArgs,
    ) -> anyhow::Result<Value> {
        let rows = &budget.rows;
        let used_tokens: u64 = rows
            .iter()
            .map(|r| token_estimate_for_context(&r.source) as u64)
            .sum();

        let include_report = args.include_budget_report == Some(true);
        let budget_tokens_json = args
            .max_tokens
            .map(|v| json!(v))
            .unwrap_or(json!(used_tokens));

        let budget_report = if include_report {
            Some(json!({
                "budget_tokens": budget_tokens_json,
                "used_tokens": used_tokens,
                "included_symbols": budget.included_symbols,
                "excluded_symbols": budget.excluded_symbols,
                "strategy": strategy_label.as_deref().unwrap_or("full"),
            }))
        } else {
            None
        };

        if rows.is_empty() {
            return self.shape_empty_response(sym_refs, store, include_report, budget_report);
        }

        let multi_shape = sym_refs.len() > 1 || include_report;
        let body = if !multi_shape {
            let r0 = &rows[0];
            let mut o = json!({
                "mode": "context_bundle",
                "symbol_id": r0.sym_id,
                "name": r0.name,
                "signature": r0.signature,
                "kind": r0.kind,
                "source": r0.source.clone(),
                "imports": r0.imports,
                "file": r0.file.clone(),
            });
            if let Some(br) = budget_report
                && let Value::Object(ref mut m) = o
            {
                m.insert("budget_report".to_string(), br);
            }
            o
        } else {
            let symbols: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "symbol_id": r.sym_id,
                        "name": r.name,
                        "signature": r.signature,
                        "kind": r.kind,
                        "source": r.source,
                        "imports": r.imports,
                        "file": r.file,
                    })
                })
                .collect();
            let mut o = json!({
                "mode": "context_bundle",
                "symbols": symbols,
            });
            if let Some(br) = budget_report
                && let Value::Object(ref mut m) = o
            {
                m.insert("budget_report".to_string(), br);
            }
            o
        };

        let raw_bytes = serde_json::to_string(&body)
            .map(|s| s.len() as u64)
            .unwrap_or(1)
            .max(1);
        let timer = MetaBuilder::new(raw_bytes);
        self.success_response(&timer, body)
    }

    fn shape_empty_response(
        &self,
        sym_refs: &[&ContentRef],
        store: &IndexStore,
        include_report: bool,
        budget_report: Option<Value>,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(1);
        if include_report {
            let body = json!({
                "mode": "context_bundle",
                "symbols": [],
                "budget_report": budget_report.unwrap_or(json!({})),
            });
            return self.success_response(&timer, body);
        }
        let fid = match resolve_symbol_id(
            store,
            sym_refs[0].stable_id.as_deref(),
            sym_refs[0].id.as_deref(),
            sym_refs[0].path.as_deref(),
            sym_refs[0].start_line,
        ) {
            Ok(Some(s)) => s,
            Ok(None) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "symbol ref needs id/stable_id, or path + start_line.".to_string(),
                        detail: json!({}),
                    },
                );
            }
            Err(e) => return self.error_response(&timer, index_error(e)),
        };
        let body = json!({
            "mode": "context_bundle",
            "symbol_id": fid,
            "source": "",
            "name": "",
            "signature": "",
            "kind": Value::Null,
            "imports": [],
            "file": "",
        });
        self.success_response(&timer, body)
    }
}
