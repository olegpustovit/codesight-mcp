use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::agent_v1::util::*;
use crate::service::args::GetSectionArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

impl CodeService {
    pub(super) fn context_section(
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
                    message: "section_context requires refs with at least one section ref."
                        .to_string(),
                    detail: json!({}),
                },
            );
        };
        let sec_pref = refs
            .iter()
            .find(|r| r.kind.eq_ignore_ascii_case("section"))
            .or_else(|| refs.first());
        let Some(pref) = sec_pref else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "section_context requires a non-empty refs list.".to_string(),
                    detail: json!({}),
                },
            );
        };
        if !pref.kind.eq_ignore_ascii_case("section") {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "section_context requires kind \"section\" on the target ref."
                        .to_string(),
                    detail: json!({ "kind": pref.kind }),
                },
            );
        }
        let Some(sec_id) = pref
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
                    message: "section ref needs id or stable_id.".to_string(),
                    detail: json!({}),
                },
            );
        };

        let store = require_store!(doc: self, &args.source_id, args.repo_path.as_deref(), timer);

        let get = GetSectionArgs {
            source_id: args.source_id.clone(),
            section_id: Some(sec_id.clone()),
            id: None,
            verify: false,
            repo_path: args.repo_path.clone(),
        };
        let primary_res = self.get_section(get)?;
        if primary_res.get("ok").and_then(|b| b.as_bool()) != Some(true) {
            return Ok(primary_res);
        }

        let mut section_json = primary_res["data"].clone();
        if let Some(mt) = args.max_tokens
            && let Some(obj) = section_json.as_object_mut()
            && let Some(Value::String(content)) = obj.get("content")
        {
            let (c, trunc) = truncate_content_to_token_budget(content, mt);
            obj.insert("content".to_string(), json!(c));
            if trunc {
                obj.insert("content_truncated".to_string(), json!(true));
            }
        }

        let file = section_json
            .get("file")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let max_neighbors = args.limit.clamp(0, 20);
        let mut warnings = Vec::new();

        let (ancestors, children, neighbors) = if file.is_empty() {
            (Vec::new(), Vec::new(), Vec::new())
        } else {
            match store.sections_for_file(file) {
                Ok(all) => {
                    let heading_path = all
                        .iter()
                        .find(|row| row.get("id").and_then(|x| x.as_str()) == Some(sec_id.as_str()))
                        .and_then(|row| row.get("heading_path").and_then(|x| x.as_str()))
                        .unwrap_or("");
                    let ancestors = heading_path_ancestors(&all, heading_path);
                    let children = if args.include_children {
                        direct_child_sections(&all, heading_path)
                    } else {
                        Vec::new()
                    };
                    let neighbors =
                        outline_neighbors(&all, sec_id.as_str(), max_neighbors, &args.source_id);
                    (ancestors, children, neighbors)
                }
                Err(e) => {
                    warnings.push(json!({
                        "code": "NEIGHBOR_ENUM_FAILED",
                        "message": format!("could not load outline for section_context: {e:#}"),
                    }));
                    (Vec::new(), Vec::new(), Vec::new())
                }
            }
        };

        let body = json!({
            "mode": "section_context",
            "methodology": "heading_tree_neighbors_same_file",
            "section": section_json,
            "ancestors": ancestors,
            "children": children,
            "neighbors": neighbors,
        });
        // ROADMAP P1.1: baseline = full-file read of the doc containing the
        // target section (naive alternative is "open the markdown and read
        // end-to-end"). Falls back to 0 (→ `tokens_saved: null`) when the
        // section has no resolvable file.
        let raw_bytes = if file.is_empty() {
            0
        } else {
            store.sum_file_sizes_for_paths(&[file.to_string()]).unwrap_or(0)
        };
        let timer = MetaBuilder::new(raw_bytes).with_confidence(Confidence::Safe);
        let mut response = self.success_response_with_warnings(&timer, body, warnings)?;
        if let Value::Object(ref mut top) = response {
            let n_anc = top
                .get("data")
                .and_then(|d| d.get("ancestors"))
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let n_ch = top
                .get("data")
                .and_then(|d| d.get("children"))
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if let Some(meta_val) = top.get_mut("_meta")
                && let Some(meta) = meta_val.as_object_mut()
            {
                meta.insert("ancestor_count".to_string(), json!(n_anc));
                meta.insert("child_count".to_string(), json!(n_ch));
                if let Some(t) = meta.get("timing_ms").cloned() {
                    meta.insert("latency_ms".to_string(), t);
                }
            }
        }
        Ok(response)
    }
}
