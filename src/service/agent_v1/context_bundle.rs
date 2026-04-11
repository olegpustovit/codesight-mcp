//! `assemble_context` mode: `context_bundle`.

use std::fs;

use serde_json::{Value, json};

use crate::core::symbols::SymbolKind;
use crate::indexing::imports::context_bundle_import_lines;
use crate::service::CodeService;
use crate::service::agent_v1::args::{AssembleContextArgs, ContentRef};
use crate::service::agent_v1::util::*;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
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

        let sym_refs: Vec<&ContentRef> = refs
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
            None::<String>
        } else {
            Some(bs_raw.to_ascii_lowercase())
        };

        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };

        #[derive(Clone)]
        struct BundleRow {
            sym_id: String,
            name: String,
            signature: String,
            kind: SymbolKind,
            file: String,
            imports: Vec<String>,
            source: String,
        }

        let mut rows: Vec<BundleRow> = Vec::new();
        for pref in &sym_refs {
            let Some(sym_id) = pref
                .stable_id
                .clone()
                .or(pref.id.clone())
                .filter(|s| !s.is_empty())
            else {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "symbol ref needs id or stable_id.".to_string(),
                        detail: json!({}),
                    },
                );
            };

            let sym = match self.retrieve_symbol(&store, &sym_id, false, 0) {
                Ok(s) => s,
                Err(e) => return self.error_response(timer, e),
            };

            let name = store
                .symbol_names_by_id(&sym_id)
                .map_err(index_error)?
                .map(|(n, _)| n)
                .unwrap_or_else(|| {
                    sym_id
                        .rsplit("::")
                        .next()
                        .unwrap_or(&sym_id)
                        .split('#')
                        .next()
                        .unwrap_or(&sym_id)
                        .to_string()
                });

            let signature = store
                .symbol_signature_by_id(&sym_id)
                .map_err(index_error)?
                .unwrap_or_default();

            let lang = store
                .file_language(&sym.file)
                .map_err(index_error)?
                .unwrap_or_else(|| "python".to_string());

            let root = store.root_path().map_err(index_error)?;
            let file_abs = root.join(&sym.file);
            let file_content = match fs::read_to_string(&file_abs) {
                Ok(c) => c,
                Err(_) => {
                    return self.error_response(
                        timer,
                        ToolError {
                            category: ToolErrorCategory::NotFound,
                            code: "NOT_FOUND",
                            message: format!("File '{}' is missing from disk.", sym.file),
                            detail: json!({ "file": sym.file }),
                        },
                    );
                }
            };
            let imports = context_bundle_import_lines(&file_content, &sym.file, &lang);

            rows.push(BundleRow {
                sym_id,
                name,
                signature,
                kind: sym.kind,
                file: sym.file,
                imports,
                source: sym.source,
            });
        }

        let mut included_symbols: Vec<String> = Vec::new();
        let mut excluded_symbols: Vec<String> = Vec::new();

        if let Some(bt) = args.max_tokens {
            let mut kept: Vec<BundleRow> = Vec::new();
            let mut used_tokens_pack: u64 = 0;
            for row in rows {
                let t = token_estimate_for_context(&row.source) as u64;
                if used_tokens_pack.saturating_add(t) > bt as u64 {
                    excluded_symbols.push(row.sym_id.clone());
                } else {
                    used_tokens_pack = used_tokens_pack.saturating_add(t);
                    included_symbols.push(row.sym_id.clone());
                    kept.push(row);
                }
            }
            rows = kept;
        } else {
            for row in &rows {
                included_symbols.push(row.sym_id.clone());
            }
        }

        if strategy_label.as_deref() == Some("compact") {
            for r in &mut rows {
                r.source.clear();
            }
        }

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
                "included_symbols": included_symbols,
                "excluded_symbols": excluded_symbols,
                "strategy": strategy_label.as_deref().unwrap_or("full"),
            }))
        } else {
            None
        };

        if rows.is_empty() {
            let timer = MetaBuilder::new(1);
            if include_report {
                let body = json!({
                    "mode": "context_bundle",
                    "symbols": [],
                    "budget_report": budget_report.unwrap_or(json!({})),
                });
                return self.success_response(&timer, body);
            }
            let Some(fid) = sym_refs[0]
                .stable_id
                .clone()
                .or(sym_refs[0].id.clone())
                .filter(|s| !s.is_empty())
            else {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "symbol ref needs id or stable_id.".to_string(),
                        detail: json!({}),
                    },
                );
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
            return self.success_response(&timer, body);
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
}
