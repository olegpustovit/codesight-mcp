use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::helpers::index_error;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn context_workspace_digest(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };
        let raw_bytes = store.repo_total_bytes().unwrap_or(0).max(1);
        let timer_overall = MetaBuilder::new(raw_bytes);
        let summary = match store.repo_summary() {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let mut language_distribution = serde_json::Map::new();
        for (lang, n) in &summary.languages {
            language_distribution.insert(lang.clone(), json!(n));
        }
        let kind_distribution = match store.kind_distribution() {
            Ok(m) => serde_json::to_value(m).unwrap_or(json!({})),
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let most_central_files: Vec<Value> = {
            let adj = store.dependency_graph().unwrap_or_default();
            if adj.is_empty() {
                Vec::new()
            } else {
                let pr = crate::analysis::graph::compute_pagerank(&adj, 0.85, 100, 1e-6);
                let mut ranked: Vec<_> = pr
                    .scores
                    .iter()
                    .filter(|(node, _)| {
                        !codesight_storage::storage::is_outline_noise(node.as_str())
                    })
                    .collect();
                ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
                ranked
                    .into_iter()
                    .take(10)
                    .map(|(node, score)| json!({ "kind": "file", "path": node, "score": score }))
                    .collect()
            }
        };
        let most_imported_files: Vec<Value> = match store.most_imported_files(10) {
            Ok(rows) => rows
                .into_iter()
                .map(|(file, n)| json!({ "file": file, "imported_by": n }))
                .collect(),
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let example_queries: Vec<Value> = match store.symbol_samples_for_suggestions(20) {
            Ok(samples) => {
                let mut out = vec![json!({
                    "query": format!("symbols in {}", summary.repo),
                    "tool": "search_catalog",
                    "description": "Run a hybrid catalog search over this indexed workspace.",
                })];
                for (name, kind, file_path) in samples {
                    if out.len() >= 6 {
                        break;
                    }
                    if codesight_storage::storage::is_outline_noise(&file_path) {
                        continue;
                    }
                    out.push(json!({
                        "query": name.clone(),
                        "tool": "search_catalog",
                        "description": format!("Find indexed {kind} `{name}`."),
                    }));
                }
                out
            }
            Err(e) => return self.error_response(timer, index_error(e)),
        };
        let body = json!({
            "mode": "workspace_digest",
            "repo": summary.repo,
            "symbol_count": summary.symbol_count,
            "file_count": summary.file_count,
            "language_distribution": language_distribution,
            "kind_distribution": kind_distribution,
            "most_central_files": most_central_files,
            "most_imported_files": most_imported_files,
            "example_queries": example_queries,
        });
        self.success_response(&timer_overall, body)
    }
}
