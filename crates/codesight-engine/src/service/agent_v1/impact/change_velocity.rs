//! `evaluate_impact analysis=change_velocity` — git-log-driven churn summary.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_change_velocity(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);

        let target_owned = args
            .target
            .as_deref()
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                args.refs.as_ref().and_then(|refs| {
                    refs.first().and_then(|r| {
                        r.stable_id
                            .clone()
                            .or_else(|| r.id.clone())
                            .or_else(|| r.path.clone())
                            .filter(|s| !s.is_empty())
                    })
                })
            });
        let Some(target) = target_owned else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "change_velocity requires `target` (a file path or symbol id) or \
                              at least one entry in `refs`."
                        .to_string(),
                    detail: json!({ "analysis": "change_velocity" }),
                },
            );
        };
        let target = target.as_str();
        let days = args.days.unwrap_or(90);

        let root = match store.root_path() {
            Ok(r) => r,
            Err(_) => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "NOT_LOCAL",
                        message: "Source is not a local repository.".to_string(),
                        detail: json!({}),
                    },
                );
            }
        };

        let is_symbol = target.contains("::")
            || target.contains('#')
            || store.symbol_record(target).ok().flatten().is_some();
        let (target_type, file_path, symbol_name) = if is_symbol {
            let rec = store.symbol_record(target)?;
            match rec {
                Some(r) => {
                    let name =
                        r.id.rsplit("::")
                            .next()
                            .unwrap_or("")
                            .split('#')
                            .next()
                            .unwrap_or("")
                            .to_string();
                    ("symbol".to_string(), r.file_path.clone(), Some(name))
                }
                None => {
                    let results = store.search_symbols_name_contains(target, None, None, 1)?;
                    if let Some(sym) = results.first() {
                        (
                            "symbol".to_string(),
                            sym.file.clone(),
                            Some(sym.name.clone()),
                        )
                    } else {
                        return self.error_response(
                            &timer,
                            ToolError {
                                category: ToolErrorCategory::NotFound,
                                code: "NOT_FOUND",
                                message: format!("Target '{target}' not found."),
                                detail: json!({ "target": target }),
                            },
                        );
                    }
                }
            }
        } else {
            ("file".to_string(), target.to_string(), None)
        };

        let git_result = std::process::Command::new("git")
            .args([
                "log",
                "--oneline",
                &format!("--since={days} days ago"),
                "--follow",
                "--",
                &file_path,
            ])
            .current_dir(&root)
            .output();

        let (commits, authors, churn_per_week) = match git_result {
            Ok(o) if o.status.success() => {
                let log = String::from_utf8_lossy(&o.stdout);
                let commit_count = log.lines().filter(|l| !l.is_empty()).count();

                let author_output = std::process::Command::new("git")
                    .args([
                        "log",
                        &format!("--since={days} days ago"),
                        "--format=%an",
                        "--follow",
                        "--",
                        &file_path,
                    ])
                    .current_dir(&root)
                    .output()
                    .ok();
                let authors = author_output
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .filter(|l| !l.is_empty())
                            .collect::<HashSet<_>>()
                            .len()
                    })
                    .unwrap_or(0);

                let weeks = (days as f64 / 7.0).max(1.0);
                let cpw = commit_count as f64 / weeks;
                (commit_count, authors, cpw)
            }
            _ => (0, 0, 0.0),
        };

        let assessment = if churn_per_week < 0.5 {
            "stable"
        } else if churn_per_week < 2.0 {
            "active"
        } else {
            "volatile"
        };

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        let mut data = json!({
            "analysis": "change_velocity",
            "target_type": target_type,
            "file": file_path,
            "commits": commits,
            "authors": authors,
            "churn_per_week": churn_per_week,
            "assessment": assessment,
            "days": days,
        });
        if let Some(sname) = &symbol_name {
            data["symbol_name"] = json!(sname);
        }

        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            vec![
                ("methodology".to_string(), json!("git_log")),
                ("confidence_level".to_string(), json!("high")),
            ],
        )
    }
}
