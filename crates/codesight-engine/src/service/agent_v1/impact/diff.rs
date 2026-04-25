//! `evaluate_impact analysis=diff` — line-oriented text diff between two indexed symbol spans.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{ContentRef, EvaluateImpactArgs};
use crate::service::agent_v1::util::{
    graph::simple_text_diff, ref_requires_strict_resolution, resolve_symbol_id_strict,
};
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

fn resolve_diff_ref(
    store: &codesight_storage::storage::IndexStore,
    r: &ContentRef,
) -> anyhow::Result<Result<(String, &'static str), ToolError>> {
    if !r.kind.eq_ignore_ascii_case("symbol") {
        return Ok(Err(ToolError {
            category: ToolErrorCategory::InvalidArgument,
            code: "INVALID_REQUEST",
            message: "diff requires refs with kind=symbol.".to_string(),
            detail: json!({ "ref": r }),
        }));
    }
    let is_strict = ref_requires_strict_resolution(
        r.stable_id.as_deref(),
        r.id.as_deref(),
        r.path.as_deref(),
        r.start_line,
        r.symbol_name.as_deref(),
    );
    let resolved = resolve_symbol_id_strict(
        store,
        r.stable_id.as_deref(),
        r.id.as_deref(),
        r.path.as_deref(),
        r.start_line,
        r.symbol_name.as_deref(),
    )?;
    match resolved {
        // ROADMAP P0.2 #3: bubble up the resolution path so the caller can
        // echo it in `_meta.resolution`.
        Some((id, strategy)) => Ok(Ok((id, strategy.as_str()))),
        None if is_strict => Ok(Err(ToolError {
            category: ToolErrorCategory::NotFound,
            code: "NOT_FOUND",
            message: "diff could not resolve ref.".to_string(),
            detail: json!({ "unresolved": [r] }),
        })),
        None => Ok(Err(ToolError {
            category: ToolErrorCategory::InvalidArgument,
            code: "INVALID_REQUEST",
            message: "diff ref provided no resolvable fields.".to_string(),
            detail: json!({ "ref": r }),
        })),
    }
}

impl CodeService {
    pub(super) fn evaluate_impact_diff(&self, args: &EvaluateImpactArgs) -> anyhow::Result<Value> {
        let timer = MetaBuilder::new(0);

        let Some(refs) = args.refs.as_ref() else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff requires refs array.".to_string(),
                    detail: json!({}),
                },
            );
        };
        if refs.len() < 2 {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "diff requires two symbol refs.".to_string(),
                    detail: json!({}),
                },
            );
        }
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &timer);
        let (id_a, res_a) = match resolve_diff_ref(&store, &refs[0])? {
            Ok(pair) => pair,
            Err(e) => return self.error_response(&timer, e),
        };
        let (id_b, res_b) = match resolve_diff_ref(&store, &refs[1])? {
            Ok(pair) => pair,
            Err(e) => return self.error_response(&timer, e),
        };
        let ra = store.symbol_record(&id_a)?;
        let rb = store.symbol_record(&id_b)?;
        let (Some(ra), Some(rb)) = (ra, rb) else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: "One or both symbols missing from index.".to_string(),
                    detail: json!({ "id_a": id_a, "id_b": id_b }),
                },
            );
        };
        // ROADMAP P3.1: cross-file diff is supported — the analysis is a
        // text diff of two indexed symbol spans, and the spans are
        // self-contained. Before this, diffing two impls of the same trait
        // method that lived in different files required copying the source
        // out of the index manually. Baseline size stays the larger of the
        // two files (honest for the full-file counterfactual).
        let raw_bytes = ra.file_size_bytes.max(rb.file_size_bytes);
        let timer = MetaBuilder::new(raw_bytes).with_confidence(Confidence::Safe);
        let patch = simple_text_diff(&ra.stored_source, &rb.stored_source);
        let mut finding = json!({
            "finding_id": "diff:symbol_pair",
            "analysis": "diff",
            "severity": "info",
            "confidence": 1.0,
            "summary": "line-oriented text diff of indexed symbol spans",
            "evidence": patch,
            "related_refs": [id_a, id_b],
        });
        if ra.file_path != rb.file_path
            && let Value::Object(map) = &mut finding
        {
            map.insert(
                "cross_file".to_string(),
                json!({ "file_a": ra.file_path, "file_b": rb.file_path }),
            );
        }
        // ROADMAP P0.2 #3: echo both ref resolutions in `_meta`.
        self.success_response_with_meta_extra(
            &timer,
            json!({
                "analysis": "diff",
                "findings": [finding],
            }),
            Vec::new(),
            [(
                "resolution".to_string(),
                json!({ "ref_a": res_a, "ref_b": res_b }),
            )],
        )
    }
}
