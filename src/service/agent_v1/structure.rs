//! Repo/file structure views.
//!
//! The public entry point is `CodeService::inspect_structure`. Each supported
//! `view` is implemented as a private method on `CodeService` so individual
//! views can be read, reviewed, and tested without wading through an
//! 8-branch dispatcher.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{DEFAULT_FILE_TREE_MAX_FILES, InspectStructureArgs};
use crate::service::agent_v1::file_tree::{
    FileTreeBuildNode, file_tree_hint, file_tree_insert, file_tree_to_json,
    normalize_file_tree_path_prefix,
};
use crate::service::agent_v1::util::pack_catalog_items_to_token_budget;
use crate::service::agent_v1::util_graph::build_toc_tree;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

/// Default token budget applied to `inspect_structure view=repo_outline`
/// when the caller does not pass `max_tokens`. Sized to stay well under
/// typical harness response limits on repos that index hundreds of files.
const DEFAULT_REPO_OUTLINE_TOKEN_BUDGET: u32 = 8000;

/// Default budget for `view=toc`, `view=toc_tree`, and `view=document_outline`.
/// Large documents (e.g. REFACTOR_PLAN.md) can produce 30k+ token TOCs if
/// unbounded. Same pattern as `DEFAULT_REPO_OUTLINE_TOKEN_BUDGET`.
const DEFAULT_TOC_TOKEN_BUDGET: u32 = 8000;

/// Result of a store-open attempt: either the ready pair or a pre-formed
/// error envelope the caller should return verbatim.
type StoreOrError<S> = Result<(S, MetaBuilder), Value>;

impl CodeService {
    pub(in crate::service) fn inspect_structure(&self, v: Value) -> anyhow::Result<Value> {
        let args: InspectStructureArgs = ValidatedArgs::from_validated(v)?.into_inner();
        match args.view.to_ascii_lowercase().as_str() {
            "file_tree" => self.structure_file_tree(&args),
            "repo_outline" => self.structure_repo_outline(&args),
            "repository_rollup" => self.structure_repository_rollup(&args),
            "file_outline" => self.structure_file_outline(&args),
            "toc" => self.structure_toc(&args),
            "toc_tree" => self.structure_toc_tree(&args),
            "document_outline" => self.structure_document_outline(&args),
            "class_hierarchy" => self.structure_class_hierarchy(&args),
            other => self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!("Unknown inspect_structure view '{other}'."),
                    detail: json!({ "view": args.view }),
                },
            ),
        }
    }

    /// Open the code index for this call and prepare a timer. Returns a
    /// pre-formed error envelope on failure so callers can early-return
    /// with no further ceremony.
    fn setup_repo_call(
        &self,
        args: &InspectStructureArgs,
    ) -> anyhow::Result<StoreOrError<IndexStore>> {
        match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => {
                let raw_bytes = store.repo_total_bytes()?;
                Ok(Ok((store, MetaBuilder::new(raw_bytes))))
            }
            Err(e) => Ok(Err(self.error_response(&MetaBuilder::new(0), e)?)),
        }
    }

    /// Open the doc index for this call and prepare a timer. Same shape as
    /// `setup_repo_call`.
    fn setup_doc_call(
        &self,
        args: &InspectStructureArgs,
    ) -> anyhow::Result<StoreOrError<DocStore>> {
        match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => {
                let raw_bytes = store.repo_total_bytes()?;
                Ok(Ok((store, MetaBuilder::new(raw_bytes))))
            }
            Err(e) => Ok(Err(self.error_response(&MetaBuilder::new(0), e)?)),
        }
    }

    fn structure_file_tree(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let mut max_files = args.max_files.unwrap_or(DEFAULT_FILE_TREE_MAX_FILES).max(1);
        // Apply token budget as a row cap. Each tree entry serializes to
        // roughly 120 bytes (path + language + size + symbol_count);
        // take the stricter of max_files and max_tokens.
        if let Some(budget) = args.max_tokens {
            let budget_rows =
                (budget as usize).saturating_mul(crate::config::BYTES_PER_TOKEN as usize) / 120;
            max_files = max_files.min(budget_rows.max(1));
        }
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
            // Heuristic: .h files reported as cpp may actually be C. If a .h
            // file has symbols but none are C++ specific (class, method,
            // namespace, type), reclassify as "c". Files with no symbols keep
            // their default "cpp" classification.
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
            Vec::new(),
            [("file_count".to_string(), json!(listed))],
        )
    }

    fn structure_repo_outline(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let outline = store.repo_outline()?;
        let budget = args.max_tokens.unwrap_or(DEFAULT_REPO_OUTLINE_TOKEN_BUDGET);
        let (outline, truncated) = pack_catalog_items_to_token_budget(outline, budget);
        // Compute most_central_symbols via dependency graph PageRank. Nodes
        // in this graph are file paths, so emit file-shaped entries (no
        // synthetic `symbol_id` — clients that want to round-trip into
        // `read_content` should pass `kind: "file"` with `path`). Noise
        // files (build.rs, .mcp.json, …) are filtered so config artifacts
        // don't dominate the ranking.
        let adj = store.dependency_graph().unwrap_or_default();
        let most_central = if adj.is_empty() {
            Vec::new()
        } else {
            let pr = codesight_analysis::graph::compute_pagerank(&adj, 0.85, 100, 1e-6);
            let mut ranked: Vec<_> = pr
                .scores
                .iter()
                .filter(|(node, _)| !codesight_storage::storage::is_outline_noise(node.as_str()))
                .collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            ranked
                .iter()
                .take(5)
                .map(|(node, score)| json!({ "kind": "file", "path": node, "score": score }))
                .collect()
        };
        self.success_response(
            &timer,
            json!({
                "view": "repo_outline",
                "items": outline,
                "most_central_symbols": most_central,
                "truncated": truncated,
            }),
        )
    }

    fn structure_repository_rollup(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let rollup = store.repository_rollup()?;
        let mut body = serde_json::Map::new();
        body.insert("view".to_string(), json!("repository_rollup"));
        if let Value::Object(m) = rollup {
            body.extend(m);
        }
        self.success_response(&timer, Value::Object(body))
    }

    fn structure_file_outline(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let fp = args.file_path.as_ref().or(args.path.as_ref());
        if fp.is_some() && args.file_paths.is_some() {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "file_outline: pass either file_path/path or file_paths, not both."
                        .to_string(),
                    detail: json!({}),
                },
            );
        }
        if let Some(paths) = args.file_paths.as_ref() {
            let mut results = Vec::with_capacity(paths.len());
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
        let Some(p) = fp else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "file_outline requires path, file_path, or file_paths.".to_string(),
                    detail: json!({}),
                },
            );
        };
        // Refine the timer now that we know the target file size; single-file
        // outlines should charge only the file itself, not the whole repo.
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

    fn structure_toc(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_doc_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let toc = store.toc()?;
        let budget = args.max_tokens.unwrap_or(DEFAULT_TOC_TOKEN_BUDGET);
        let (toc, truncated) = pack_catalog_items_to_token_budget(toc, budget);
        self.success_response(
            &timer,
            json!({ "view": "toc", "items": toc, "truncated": truncated }),
        )
    }

    fn structure_toc_tree(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_doc_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let budget = args.max_tokens.unwrap_or(DEFAULT_TOC_TOKEN_BUDGET);
        // Trim the flat TOC to the budget, then build the tree from the
        // trimmed prefix. Packing the flat list keeps the budget predictable
        // and still preserves file-adjacency ordering in the resulting tree.
        let flat = store.toc()?;
        let (flat, truncated) = pack_catalog_items_to_token_budget(flat, budget);
        let tree = build_toc_tree(&flat);
        self.success_response(
            &timer,
            json!({ "view": "toc_tree", "items": tree, "truncated": truncated }),
        )
    }

    fn structure_document_outline(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_doc_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let Some(file_path) = args.file_path.as_ref().or(args.path.as_ref()) else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
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
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Document '{file_path}' is not indexed."),
                    detail: json!({ "path": file_path }),
                },
            );
        }
        let budget = args.max_tokens.unwrap_or(DEFAULT_TOC_TOKEN_BUDGET);
        let (sections, truncated) = pack_catalog_items_to_token_budget(sections, budget);
        self.success_response(
            &timer,
            json!({ "view": "document_outline", "items": sections, "truncated": truncated }),
        )
    }

    fn structure_class_hierarchy(&self, args: &InspectStructureArgs) -> anyhow::Result<Value> {
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        let fp = args.file_path.as_ref().or(args.path.as_ref());
        let class_name = match args
            .class_path
            .as_ref()
            .or(fp)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            Some(n) => n.to_string(),
            None => {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::InvalidArgument,
                        code: "INVALID_REQUEST",
                        message: "class_hierarchy requires class_path (the class name)."
                            .to_string(),
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
                        category: ToolErrorCategory::Internal,
                        code: "INDEX_ERROR",
                        message: e.to_string(),
                        detail: json!({}),
                    },
                );
            }
        };
        let Some(view) =
            codesight_storage::class_hierarchy::compute_hierarchy_view(&rows, &class_name)
        else {
            return self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::NotFound,
                    code: "NOT_FOUND",
                    message: format!("Class '{class_name}' not found in index."),
                    detail: json!({ "class_name": class_name }),
                },
            );
        };
        let mut body = json!({
            "view": "class_hierarchy",
            "class": view.class,
            "ancestors": view.ancestors,
            "descendants": view.descendants,
            "ancestor_count": view.ancestor_count,
            "descendant_count": view.descendant_count,
        });
        // Rust, Go, and similarly non-OOP languages don't model ancestors /
        // descendants via class inheritance — traits and interfaces live in
        // a separate graph. When the class was found but both sides are
        // empty, emit a diagnostic note so clients don't confuse "no edges"
        // with "class not found".
        if view.ancestor_count == 0 && view.descendant_count == 0 {
            let file = view.class.file.as_str();
            let ext = std::path::Path::new(file)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let note = match ext {
                "rs" => Some(
                    "Rust structs do not use class inheritance. Trait impls and \
                     trait bounds live in a separate graph — use `inspect_graph \
                     graph=references` or `graph=related` against the target symbol \
                     instead.",
                ),
                "go" => Some(
                    "Go types do not use class inheritance. Interface \
                     satisfaction is structural — use `inspect_graph` to trace \
                     method and embedding relationships.",
                ),
                _ => None,
            };
            if let Some(msg) = note
                && let Some(obj) = body.as_object_mut()
            {
                obj.insert("note".to_string(), json!(msg));
                obj.insert("supports_hierarchy".to_string(), json!(false));
            }
        }
        self.success_response(&timer, body)
    }
}
