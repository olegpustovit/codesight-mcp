//! Remaining `assemble_context` modes (section_context, query_suggestions, …).

use serde_json::{Value, json};

use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::AssembleContextArgs;
use crate::service::agent_v1::util::*;
use crate::service::args::GetSectionArgs;
use crate::service::helpers::{ToolError, index_error};

impl CodeService {
    pub(in crate::service) fn assemble_context_tail(
        &self,
        args: &AssembleContextArgs,
        mode: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        match mode {
            "section_context" => {
                let Some(refs) = args.refs.as_ref().filter(|r| !r.is_empty()) else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "section_context requires refs with at least one section ref."
                                .to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let sec_pref = refs
                    .iter()
                    .find(|r| r.kind.to_ascii_lowercase() == "section")
                    .or_else(|| refs.first());
                let Some(pref) = sec_pref else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "section_context requires a non-empty refs list.".to_string(),
                            detail: json!({}),
                        },
                    );
                };
                if pref.kind.to_ascii_lowercase() != "section" {
                    return self.error_response(
                        &timer,
                        ToolError {
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
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "section ref needs id or stable_id.".to_string(),
                            detail: json!({}),
                        },
                    );
                };

                let store = match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };

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
                if let Some(mt) = args.max_tokens {
                    if let Some(obj) = section_json.as_object_mut() {
                        if let Some(Value::String(content)) = obj.get("content") {
                            let (c, trunc) = truncate_content_to_token_budget(content, mt);
                            obj.insert("content".to_string(), json!(c));
                            if trunc {
                                obj.insert("content_truncated".to_string(), json!(true));
                            }
                        }
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
                                .find(|row| {
                                    row.get("id").and_then(|x| x.as_str()) == Some(sec_id.as_str())
                                })
                                .and_then(|row| row.get("heading_path").and_then(|x| x.as_str()))
                                .unwrap_or("");
                            let ancestors = heading_path_ancestors(&all, heading_path);
                            let children = if args.include_children {
                                direct_child_sections(&all, heading_path)
                            } else {
                                Vec::new()
                            };
                            let neighbors = outline_neighbors(
                                &all,
                                sec_id.as_str(),
                                max_neighbors,
                                &args.source_id,
                            );
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
                let raw_bytes = serde_json::to_string(&body)
                    .map(|s| s.len() as u64)
                    .unwrap_or(1)
                    .max(1);
                let timer = MetaBuilder::new(raw_bytes);
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
                    if let Some(meta_val) = top.get_mut("_meta") {
                        if let Some(meta) = meta_val.as_object_mut() {
                            meta.insert("ancestor_count".to_string(), json!(n_anc));
                            meta.insert("child_count".to_string(), json!(n_ch));
                            if let Some(t) = meta.get("timing_ms").cloned() {
                                meta.insert("latency_ms".to_string(), t);
                            }
                        }
                    }
                }
                Ok(response)
            }
            "query_suggestions" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, e),
                };
                let summary = match store.repo_summary() {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, index_error(e)),
                };
                if summary.symbol_count == 0 {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "EMPTY_INDEX",
                            message: "Index contains no symbols.".to_string(),
                            detail: json!({ "source_id": args.source_id }),
                        },
                    );
                }
                let kind_distribution = match store.kind_distribution() {
                    Ok(m) => serde_json::to_value(m).unwrap_or(json!({})),
                    Err(e) => return self.error_response(&timer, index_error(e)),
                };
                let mut language_distribution = serde_json::Map::new();
                for (lang, n) in &summary.languages {
                    language_distribution.insert(lang.clone(), json!(n));
                }

                let samples = match store.symbol_samples_for_suggestions(6) {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&timer, index_error(e)),
                };
                let mut example_queries: Vec<Value> = vec![json!({
                    "query": format!("symbols in {}", summary.repo),
                    "tool": "search_catalog",
                    "description": "Run a hybrid catalog search over this indexed workspace.",
                })];
                for (name, kind) in samples {
                    example_queries.push(json!({
                        "query": name.clone(),
                        "tool": "search_catalog",
                        "description": format!("Find indexed {kind} `{name}`."),
                    }));
                }

                let most_imported_files: Vec<Value> = match store.python_most_imported_files(10) {
                    Ok(rows) => rows
                        .into_iter()
                        .map(|(file, n)| json!({ "file": file, "imported_by": n }))
                        .collect(),
                    Err(e) => return self.error_response(&timer, index_error(e)),
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
                let raw_bytes = serde_json::to_string(&body)
                    .map(|s| s.len() as u64)
                    .unwrap_or(1)
                    .max(1);
                let timer = MetaBuilder::new(raw_bytes);
                self.success_response(&timer, body)
            }
            "context_bundle" => self.assemble_context_bundle(args, timer),
            "workspace_digest" => {
                let cat = self.catalog_sources(json!({}))?;
                let sources = cat["data"]["sources"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let timer = MetaBuilder::new(0);
                self.success_response(
                    &timer,
                    json!({
                        "mode": mode,
                        "sources_count": sources.len(),
                        "sources": sources,
                    }),
                )
            }
            _ => self.error_response(
                &timer,
                ToolError {
                    code: "INVALID_REQUEST",
                    message: format!("Unknown assemble_context mode '{}'.", args.mode),
                    detail: json!({ "mode": args.mode }),
                },
            ),
        }
    }
}
