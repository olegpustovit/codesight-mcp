//! E6 — `assemble_context mode=rename_plan` — ranked, token-budgeted
//! work-plan for renaming a symbol across the repo.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::config::BYTES_PER_TOKEN;
use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::agent_v1::graph_tail_b::find_references::collect_references;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn context_rename_plan(
        &self,
        args: &AssembleContextArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let old_name = args
            .old_name
            .as_deref()
            .or(args.query.as_deref())
            .map(str::to_string);
        let Some(old_name) = old_name else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "rename_plan requires `old_name` (or `query`).".into(),
                    detail: json!({}),
                },
            );
        };
        let new_name = args.new_name.clone();

        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);

        let matches = collect_references(&store, &old_name)?;

        #[derive(Default)]
        struct FileBucket {
            has_definition: bool,
            symbol_body_count: usize,
            has_text_only: bool,
            edits: Vec<Value>,
        }

        let mut buckets: BTreeMap<String, FileBucket> = BTreeMap::new();
        for m in &matches {
            let bucket = buckets.entry(m.file.clone()).or_default();
            match m.kind {
                "definition" => bucket.has_definition = true,
                "symbol_body_match" => bucket.symbol_body_count += 1,
                "text_match" => bucket.has_text_only = true,
                _ => {}
            }
            bucket.edits.push(json!({
                "kind": m.kind,
                "line": m.line,
                "preview": m.preview,
            }));
        }

        fn file_rank(b: &FileBucket) -> u8 {
            if b.has_definition {
                0
            } else if b.symbol_body_count > 0 {
                1
            } else {
                2
            }
        }

        let mut files: Vec<(String, FileBucket)> = buckets.into_iter().collect();
        files.sort_by(|a, b| {
            file_rank(&a.1)
                .cmp(&file_rank(&b.1))
                .then(b.1.symbol_body_count.cmp(&a.1.symbol_body_count))
                .then(a.0.cmp(&b.0))
        });

        let mut file_entries: Vec<Value> = Vec::new();
        let mut truncated = false;
        let mut used_tokens: u64 = 0;
        let budget = args.max_tokens.unwrap_or(0) as u64;
        for (path, bucket) in files {
            let bytes = store.file_bytes(&path)?.unwrap_or(0);
            let est_tokens = bytes / BYTES_PER_TOKEN;
            if budget > 0 && used_tokens.saturating_add(est_tokens) > budget {
                truncated = true;
                break;
            }
            used_tokens = used_tokens.saturating_add(est_tokens);
            file_entries.push(json!({
                "path": path,
                "estimated_tokens_to_read": est_tokens,
                "has_definition": bucket.has_definition,
                "symbol_body_matches": bucket.symbol_body_count,
                "edits": bucket.edits,
            }));
        }

        let mut warnings: Vec<Value> = Vec::new();
        if let Some(new_name) = new_name.as_deref() {
            // Try to resolve a single symbol with `old_name` for collision checks.
            let all = store.all_symbols_with_source()?;
            let defs: Vec<&codesight_analysis::graph::SymbolInfo> =
                all.iter().filter(|s| s.name == old_name).collect();
            if let [sym] = defs.as_slice()
                && let Ok(Some(payload)) =
                    codesight_analysis::impact::rename_safety_payload(&store, &sym.id, new_name)
                && let Some(conflicts) = payload.get("conflicts").and_then(|v| v.as_array())
                && !conflicts.is_empty()
            {
                warnings.push(json!({
                    "code": "RENAME_COLLISION",
                    "message": format!("Proposed new name '{new_name}' collides with existing symbols."),
                    "conflicts": conflicts,
                }));
            }
        }

        let data = json!({
            "mode": "rename_plan",
            "symbol": {
                "old_name": old_name,
                "new_name": new_name,
            },
            "files": file_entries,
            "warnings": warnings,
            "truncated": truncated,
            "estimated_tokens_to_read": used_tokens,
        });
        self.success_response(timer, data)
    }
}
