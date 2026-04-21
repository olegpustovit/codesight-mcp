//! `evaluate_impact analysis=changed_units` — per-symbol added/removed/modified set across two git refs.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util_graph::{
    extract_symbol_names_from_content, git_head_sha, git_show_file,
};
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn evaluate_impact_changed_units(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);

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

        if !root.is_dir() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "NOT_LOCAL",
                    message: "Source root is not accessible.".to_string(),
                    detail: json!({}),
                },
            );
        }

        let commit_refs: Vec<String> = args
            .refs
            .as_ref()
            .map(|refs| {
                refs.iter()
                    .filter(|r| r.kind.eq_ignore_ascii_case("commit"))
                    .filter_map(|r| r.id.clone().or(r.stable_id.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let (since_sha, until_sha) = if commit_refs.len() >= 2 {
            (commit_refs[0].clone(), commit_refs[1].clone())
        } else if commit_refs.len() == 1 {
            let head = git_head_sha(&root);
            match head {
                Some(h) => (commit_refs[0].clone(), h),
                None => {
                    return self.error_response(
                        &timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "GIT_ERROR",
                            message: "Cannot determine HEAD SHA.".to_string(),
                            detail: json!({}),
                        },
                    );
                }
            }
        } else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "changed_units requires refs[] with at least one commit ref, e.g. {kind: 'commit', id: '<sha>'}."
                        .to_string(),
                    detail: json!({}),
                },
            );
        };

        let diff_output = std::process::Command::new("git")
            .args(["diff", "--name-only", &since_sha, &until_sha])
            .current_dir(&root)
            .output();

        let diff_output = match diff_output {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "GIT_ERROR",
                        message: format!("git diff failed between {since_sha} and {until_sha}."),
                        detail: json!({ "since": since_sha, "until": until_sha }),
                    },
                );
            }
        };

        let changed_files: Vec<&str> = diff_output.lines().filter(|l| !l.is_empty()).collect();
        let indexer = self.indexer().clone();

        let mut added_symbols = Vec::new();
        let mut removed_symbols = Vec::new();
        let mut changed_symbols = Vec::new();

        for file in &changed_files {
            let old_content = git_show_file(&root, &since_sha, file);
            let new_content = git_show_file(&root, &until_sha, file);

            let old_syms = extract_symbol_names_from_content(
                old_content.as_deref().unwrap_or(""),
                file,
                &root,
                &indexer,
            );
            let new_syms = extract_symbol_names_from_content(
                new_content.as_deref().unwrap_or(""),
                file,
                &root,
                &indexer,
            );

            let old_map: std::collections::HashMap<&str, &str> = old_syms
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();
            let new_map: std::collections::HashMap<&str, &str> = new_syms
                .iter()
                .map(|(n, s)| (n.as_str(), s.as_str()))
                .collect();

            for (name, new_src) in &new_syms {
                if !old_map.contains_key(name.as_str()) {
                    added_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "added",
                        "blast_radius": 0,
                    }));
                } else if old_map.get(name.as_str()) != Some(&new_src.as_str()) {
                    changed_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "modified",
                        "blast_radius": 0,
                    }));
                }
            }
            for (name, _) in &old_syms {
                if !new_map.contains_key(name.as_str()) {
                    removed_symbols.push(json!({
                        "name": name,
                        "file": file,
                        "change_type": "removed",
                        "blast_radius": 0,
                    }));
                }
            }
        }

        let raw_bytes = store.repo_total_bytes().unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes);

        self.success_response(
            &timer,
            json!({
                "analysis": "changed_units",
                "added_symbols": added_symbols,
                "removed_symbols": removed_symbols,
                "changed_symbols": changed_symbols,
                "added_count": added_symbols.len(),
                "removed_count": removed_symbols.len(),
                "changed_count": changed_symbols.len(),
            }),
        )
    }
}
