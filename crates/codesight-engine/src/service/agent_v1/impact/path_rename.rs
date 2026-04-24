//! `evaluate_impact analysis=path_rename` — plan and optionally apply a
//! module-path rewrite across Rust source files.

use std::fs;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::complexity::cyclomatic_complexity_for_path;
use codesight_analysis::impact;
use codesight_storage::storage::IndexStore;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_path_rename(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);
        let from = args
            .from
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: "path_rename requires `from`.".to_string(),
                detail: json!({ "analysis": "path_rename" }),
            })?;
        let to = args
            .to
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: "path_rename requires `to`.".to_string(),
                detail: json!({ "analysis": "path_rename" }),
            })?;
        let partial_mode = args
            .handle_partial_group_imports
            .as_deref()
            .unwrap_or("warn");
        // The handler is currently Rust-only; future language support plugs
        // in by wiring `Language::path_rename_engine` and extending the arg
        // schema with a `language` selector.
        let Some(plan) = impact::preview_path_rename(
            &store,
            "rust",
            from,
            to,
            args.scope.as_deref(),
            partial_mode,
        )?
        else {
            // ROADMAP P1.1: structural analysis — null baseline.
            let timer = MetaBuilder::new(0);
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "PATH_RENAME_UNSUPPORTED",
                    message: "path_rename: no engine registered for language 'rust'".to_string(),
                    detail: json!({ "analysis": "path_rename" }),
                },
            );
        };
        let (files, changes) = impact::summarize_path_rename(&plan);
        // ROADMAP P1.1: structural analysis — null baseline.
        let timer = MetaBuilder::new(0);

        if args.apply.unwrap_or(false) && !plan.collisions.is_empty() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "PATH_RENAME_COLLISION",
                    message: "path_rename refused to apply because the rewrite would create duplicate imports.".to_string(),
                    detail: json!({ "collisions": plan.collisions }),
                },
            );
        }

        let mut complexity_budget = None;
        let mut applied = 0usize;
        if args.apply.unwrap_or(false) && !plan.files_affected.is_empty() {
            let root = store.root_path()?;
            let before_budget = complexity_budget_snapshot(&store)?;
            for file in &plan.files_affected {
                let abs_path = root.join(&file.path);
                fs::write(&abs_path, &file.updated_source)?;
            }
            self.do_index_folder(&root)?;
            let refreshed = self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())?;
            let after_budget = complexity_budget_snapshot(&refreshed)?;
            complexity_budget = Some(json!({
                "before": before_budget,
                "after": after_budget,
                "verdict": if before_budget == after_budget { "unchanged" } else { "changed" },
            }));
            applied = changes;
        }

        let mut data = json!({
            "analysis": "path_rename",
            "from": from,
            "to": to,
            "files_affected": plan.files_affected.iter().map(|file| json!({
                "path": file.path,
                "changes": file.changes,
            })).collect::<Vec<_>>(),
            "summary": {
                "files": files,
                "changes": changes,
                "unhandled": plan.unhandled,
                "collisions": plan.collisions,
                "paths": impact::affected_paths(&plan),
            },
            "applied": applied,
        });
        if let Some(scope) = args.scope.as_deref() {
            data["scope"] = json!(scope);
        }
        if let Some(ref budget) = complexity_budget {
            data["complexity_budget"] = budget.clone();
        }

        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [("methodology".to_string(), json!("byte_scan"))],
        )
    }
}

fn complexity_budget_snapshot(store: &IndexStore) -> anyhow::Result<Value> {
    let mut total_cyclomatic = 0u64;
    let mut max_per_fn = 0u64;
    for symbol in store.all_symbols_cached()?.iter() {
        let kind = symbol.kind.to_ascii_lowercase();
        if kind != "function" && kind != "method" {
            continue;
        }
        let cyclomatic =
            cyclomatic_complexity_for_path(&symbol.source_text, &symbol.file_path) as u64;
        total_cyclomatic = total_cyclomatic.saturating_add(cyclomatic);
        max_per_fn = max_per_fn.max(cyclomatic);
    }
    Ok(json!({
        "total_cyclomatic": total_cyclomatic,
        "max_per_fn": max_per_fn,
    }))
}
