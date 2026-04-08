//! Repo/file structure views.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::{DEFAULT_FILE_TREE_MAX_FILES, InspectStructureArgs};
use crate::service::agent_v1::file_tree::{
    FileTreeBuildNode, file_tree_hint, file_tree_insert, file_tree_to_json,
    normalize_file_tree_path_prefix,
};
use crate::service::agent_v1::util_graph::build_toc_tree;
use crate::service::args::parse_args;
use crate::service::helpers::ToolError;

impl CodeService {
    pub(in crate::service) fn inspect_structure(&self, v: Value) -> anyhow::Result<Value> {
        let args: InspectStructureArgs = parse_args(v)?;
        let view = args.view.to_ascii_lowercase();
        let fp = args.file_path.clone().or(args.path.clone());
        let warnings = Vec::new();

        match view.as_str() {
            "file_tree" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let max_files = args.max_files.unwrap_or(DEFAULT_FILE_TREE_MAX_FILES).max(1);
                let prefix = normalize_file_tree_path_prefix(&args.path);
                let mut rows = store.file_tree_rows()?;
                if let Some(ref pref) = prefix {
                    rows.retain(|(p, _, _, _)| p.starts_with(pref));
                }
                let total = rows.len();
                let truncated = total > max_files;
                if truncated {
                    rows.truncate(max_files);
                }
                let listed = rows.len();
                let mut root: BTreeMap<String, FileTreeBuildNode> = BTreeMap::new();
                for (path, mut lang, size, sym_count) in rows {
                    // Heuristic: .h files reported as cpp may actually be C.
                    // If a .h file has symbols but none are C++ specific (class,
                    // method, namespace, type), reclassify as "c".
                    // Files with no symbols keep their default "cpp" classification.
                    if lang == "cpp"
                        && path.ends_with(".h")
                        && sym_count > 0
                        && !store.file_has_cpp_symbols(&path).unwrap_or(true)
                    {
                        lang = "c".to_string();
                    }
                    let file_json = json!({
                        "type": "file",
                        "path": path,
                        "language": lang,
                        "size_bytes": size,
                        "symbol_count": sym_count,
                    });
                    file_tree_insert(&mut root, &path, file_json);
                }
                let items = file_tree_to_json(&root, "");
                let mut body = serde_json::Map::new();
                body.insert("view".to_string(), json!("file_tree"));
                body.insert("items".to_string(), Value::Array(items));
                if truncated {
                    body.insert("truncated".to_string(), json!(true));
                    body.insert("total_file_count".to_string(), json!(total));
                    body.insert(
                        "hint".to_string(),
                        json!(file_tree_hint(listed, total, max_files)),
                    );
                }
                self.success_response_with_meta_extra(
                    &timer,
                    Value::Object(body),
                    warnings,
                    [("file_count".to_string(), json!(listed))],
                )
            }
            "repo_outline" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let outline = store.repo_outline()?;
                // Compute most_central_symbols via dependency graph PageRank
                let adj = store.dependency_graph().unwrap_or_default();
                let most_central = if !adj.is_empty() {
                    let pr = crate::dep_graph::compute_pagerank(&adj, 0.85, 100, 1e-6);
                    let mut ranked: Vec<_> = pr.scores.iter().collect();
                    ranked
                        .sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
                    ranked.iter().take(5).map(|(node, score)| {
                        json!({ "symbol_id": node, "file": node, "score": score, "kind": "file" })
                    }).collect::<Vec<Value>>()
                } else {
                    Vec::new()
                };
                self.success_response(
                    &timer,
                    json!({ "view": "repo_outline", "items": outline, "most_central_symbols": most_central }),
                )
            }
            "repository_rollup" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let rollup = store.repository_rollup()?;
                let mut body = serde_json::Map::new();
                body.insert("view".to_string(), json!("repository_rollup"));
                if let Value::Object(m) = rollup {
                    for (k, v) in m {
                        body.insert(k, v);
                    }
                }
                self.success_response(&timer, Value::Object(body))
            }
            "file_outline" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let has_path = fp.is_some();
                if has_path && args.file_paths.is_some() {
                    let timer = MetaBuilder::new(0);
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message:
                                "file_outline: pass either file_path/path or file_paths, not both."
                                    .to_string(),
                            detail: json!({}),
                        },
                    );
                }
                if let Some(paths) = args.file_paths.as_ref() {
                    let raw_bytes = store.repo_total_bytes()?;
                    let timer = MetaBuilder::new(raw_bytes);
                    if paths.is_empty() {
                        return self.success_response(
                            &timer,
                            json!({ "view": "file_outline", "results": [] }),
                        );
                    }
                    let mut results = Vec::new();
                    for p in paths {
                        let symbols = store.file_outline(p)?;
                        let lang = store.file_language(p)?.unwrap_or_default();
                        results.push(json!({
                            "file": p,
                            "language": lang,
                            "items": symbols,
                            "symbol_count": symbols.len(),
                        }));
                    }
                    return self.success_response(
                        &timer,
                        json!({ "view": "file_outline", "results": results }),
                    );
                }
                let Some(ref p) = fp else {
                    let timer = MetaBuilder::new(0);
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "INVALID_REQUEST",
                            message: "file_outline requires path, file_path, or file_paths."
                                .to_string(),
                            detail: json!({}),
                        },
                    );
                };
                let raw_bytes = store.file_bytes(p)?.unwrap_or(0);
                let timer = MetaBuilder::new(raw_bytes);
                let symbols = store.file_outline(p)?;
                let lang = store.file_language(p)?.unwrap_or_default();
                self.success_response(
                    &timer,
                    json!({
                        "view": "file_outline",
                        "path": p,
                        "language": lang,
                        "symbol_count": symbols.len(),
                        "items": symbols,
                    }),
                )
            }
            "toc" => {
                let store = match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let toc = store.toc()?;
                self.success_response(&timer, json!({ "view": "toc", "items": toc }))
            }
            "toc_tree" | "document_outline" => {
                let store = match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                if view == "document_outline" {
                    let Some(ref file_path) = fp else {
                        return self.error_response(
                            &timer,
                            ToolError {
                                code: "INVALID_REQUEST",
                                message: "document_outline requires path or file_path.".to_string(),
                                detail: json!({}),
                            },
                        );
                    };
                    let sections = store.sections_for_file(file_path)?;
                    if sections.is_empty() && !store.doc_file_exists(file_path)? {
                        return self.error_response(
                            &timer,
                            ToolError {
                                code: "NOT_FOUND",
                                message: format!("Document '{file_path}' is not indexed."),
                                detail: json!({ "path": file_path }),
                            },
                        );
                    }
                    return self
                        .success_response(&timer, json!({ "view": view, "items": sections }));
                }
                let flat = store.toc()?;
                let tree = build_toc_tree(&flat);
                self.success_response(&timer, json!({ "view": "toc_tree", "items": tree }))
            }
            "class_hierarchy" => {
                let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(0), e),
                };
                let raw_bytes = store.repo_total_bytes()?;
                let timer = MetaBuilder::new(raw_bytes);
                let class_name = match fp.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    Some(n) => n.to_string(),
                    None => {
                        return self.error_response(
                            &timer,
                            ToolError {
                                code: "INVALID_REQUEST",
                                message: "class_hierarchy requires path (class name).".to_string(),
                                detail: json!({}),
                            },
                        );
                    }
                };
                let rows = match store.class_index_rows() {
                    Ok(r) => r,
                    Err(e) => {
                        return self.error_response(
                            &timer,
                            ToolError {
                                code: "INDEX_ERROR",
                                message: e.to_string(),
                                detail: json!({}),
                            },
                        );
                    }
                };
                let Some(view) = crate::class_hierarchy::compute_hierarchy_view(&rows, &class_name)
                else {
                    return self.error_response(
                        &timer,
                        ToolError {
                            code: "NOT_FOUND",
                            message: format!("Class '{class_name}' not found in index."),
                            detail: json!({ "class_name": class_name }),
                        },
                    );
                };
                self.success_response(
                    &timer,
                    json!({
                        "view": "class_hierarchy",
                        "class": view.class,
                        "ancestors": view.ancestors,
                        "descendants": view.descendants,
                        "ancestor_count": view.ancestor_count,
                        "descendant_count": view.descendant_count,
                    }),
                )
            }
            _ => {
                let timer = MetaBuilder::new(0);
                self.error_response(
                    &timer,
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: format!("Unknown inspect_structure view '{}'.", args.view),
                        detail: json!({ "view": args.view }),
                    },
                )
            }
        }
    }
}
