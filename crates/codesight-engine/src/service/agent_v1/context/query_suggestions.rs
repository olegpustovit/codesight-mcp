use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn context_query_suggestions(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        // ROADMAP P1.1: query_suggestions is a metadata-only view (repo
        // summary + sample queries). No sensible per-call counterfactual —
        // surface `tokens_saved: null` by passing a 0 baseline.
        let timer_overall = MetaBuilder::new(0);
        let summary = match store.repo_summary() {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        if summary.symbol_count == 0 {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "EMPTY_INDEX",
                    message: "Index contains no symbols.".to_string(),
                    detail: json!({ "source_id": args.source_id }),
                },
            );
        }
        let kind_distribution = match store.kind_distribution() {
            Ok(m) => serde_json::to_value(m).unwrap_or(json!({})),
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let mut language_distribution = serde_json::Map::new();
        for (lang, n) in &summary.languages {
            language_distribution.insert(lang.clone(), json!(n));
        }

        let samples = match store.symbol_samples_for_suggestions(20) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let mut example_queries: Vec<Value> = vec![json!({
            "query": format!("symbols in {}", summary.repo),
            "tool": "search_catalog",
            "description": "Run a hybrid catalog search over this indexed workspace.",
        })];
        let mut added = 0usize;
        for (name, kind, file_path) in samples {
            if added >= 5 {
                break;
            }
            if codesight_language_support::is_outline_noise_basename(&file_path) {
                continue;
            }
            example_queries.push(json!({
                "query": name.clone(),
                "tool": "search_catalog",
                "description": format!("Find indexed {kind} `{name}`."),
            }));
            added += 1;
        }

        let most_imported_files: Vec<Value> = match store.most_imported_files(20) {
            Ok(rows) => rows
                .into_iter()
                .filter(|(file, _)| !codesight_language_support::is_outline_noise_basename(file))
                .take(10)
                .map(|(file, n)| json!({ "file": file, "imported_by": n }))
                .collect(),
            Err(e) => return self.error_response(timer, index_error(e)),
        };

        let body = json!({
            "mode": "query_suggestions",
            "repo": summary.repo,
            "symbol_count": summary.symbol_count,
            "file_count": summary.file_count,
            "kind_distribution": kind_distribution,
            "language_distribution": language_distribution,
            "example_queries": example_queries,
            "most_imported_files": most_imported_files,
        });
        self.success_response(&timer_overall, body)
    }
}
