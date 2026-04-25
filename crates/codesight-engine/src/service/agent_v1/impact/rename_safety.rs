//! `evaluate_impact analysis=rename_safety` — refs-driven rename validator.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateImpactArgs;
use crate::service::agent_v1::util::{ref_requires_strict_resolution, resolve_symbol_id_strict};
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;
use codesight_analysis::impact;

impl CodeService {
    pub(in crate::service) fn evaluate_impact_rename_safety(
        &self,
        args: &EvaluateImpactArgs,
    ) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);

        let r = args.refs.as_ref().and_then(|refs| refs.first());

        let Some(r) = r else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires refs[0].".to_string(),
                    detail: json!({}),
                },
            );
        };

        let new_name = r.new_name.as_deref().unwrap_or("");

        if new_name.is_empty() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "rename_safety requires new_name on the ref.".to_string(),
                    detail: json!({}),
                },
            );
        }

        let is_strict = ref_requires_strict_resolution(
            r.stable_id.as_deref(),
            r.id.as_deref(),
            r.path.as_deref(),
            r.start_line,
            r.symbol_name.as_deref(),
        );
        let resolved = resolve_symbol_id_strict(
            &store,
            r.stable_id.as_deref(),
            r.id.as_deref(),
            r.path.as_deref(),
            r.start_line,
            r.symbol_name.as_deref(),
        )?;
        let (sym_id, resolution) = match resolved {
            Some((id, strategy)) => (id, Some(strategy.as_str())),
            None if is_strict => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: "rename_safety could not resolve refs[0].".to_string(),
                        detail: json!({ "unresolved": [r] }),
                    },
                );
            }
            None => (
                r.stable_id.as_deref().or(r.id.as_deref()).unwrap_or("").to_string(),
                None,
            ),
        };

        let include_vendored = args.include_vendored.unwrap_or(false)
            || args
                .scope
                .as_deref()
                .map(codesight_language_support::scope_opts_in)
                .unwrap_or(false);
        let data = match impact::rename_safety_payload(&store, &sym_id, new_name, include_vendored)? {
            Some(data) => data,
            None => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found in index."),
                        detail: json!({ "stable_id": sym_id }),
                    },
                );
            }
        };
        // ROADMAP P1.1: structural analysis — null baseline.
        let timer = MetaBuilder::new(0);
        // ROADMAP P0.2 #3: echo which ref shape resolved so agents can
        // debug silent ambiguity. Omitted when resolution fell back to the
        // permissive opaque-id path.
        let mut extras: Vec<(String, Value)> = Vec::new();
        if let Some(r) = resolution {
            extras.push(("resolution".to_string(), json!(r)));
        }
        self.success_response_with_meta_extra(&timer, data, Vec::new(), extras)
    }
}
