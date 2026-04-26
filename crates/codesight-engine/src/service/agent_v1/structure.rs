//! Repo/file structure views.
//!
//! The public entry point is `CodeService::inspect_structure`. Each supported
//! `view` is implemented as a private method on `CodeService` so individual
//! views can be read, reviewed, and tested without wading through an
//! 8-branch dispatcher.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{DEFAULT_FILE_TREE_MAX_FILES, InspectStructureArgs};
use crate::service::agent_v1::file_tree::{
    FileTreeBuildNode, file_tree_hint, file_tree_insert, file_tree_to_json_depth,
    normalize_file_tree_path_prefix,
};
use crate::service::agent_v1::util::graph::build_toc_tree;
use crate::service::agent_v1::util::pack_catalog_items_to_token_budget;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::{Confidence, MetaBuilder};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

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
    ///
    /// ROADMAP P1.1: `inspect_structure` overview views (file_tree,
    /// repo_outline, repository_rollup, toc*) have no sensible per-call
    /// counterfactual — there's no native-tool workflow that would have
    /// returned the same shape. Pass `raw_bytes: 0` so the envelope carries
    /// `tokens_saved: null` (omitted) instead of a fabricated "would have
    /// read the whole repo" number. `file_outline` rebuilds a timer with
    /// the real per-file baseline after the path resolves.
    fn setup_repo_call(
        &self,
        args: &InspectStructureArgs,
    ) -> anyhow::Result<StoreOrError<IndexStore>> {
        match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => Ok(Ok((
                store,
                MetaBuilder::new(0).with_confidence(Confidence::Safe),
            ))),
            Err(e) => Ok(Err(self.error_response(&MetaBuilder::new(0), e)?)),
        }
    }

    /// Open the doc index for this call and prepare a timer. Same shape as
    /// `setup_repo_call`; same null-baseline rule.
    fn setup_doc_call(
        &self,
        args: &InspectStructureArgs,
    ) -> anyhow::Result<StoreOrError<DocStore>> {
        match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(store) => Ok(Ok((
                store,
                MetaBuilder::new(0).with_confidence(Confidence::Safe),
            ))),
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
        // Reclassification (e.g. .h → c): bucket rows by their declared
        // language, ask each language for its distinctive symbol kinds,
        // and batch-query storage once per bucket. The language itself
        // decides whether to override (`Language::reclassify_file_language`).
        use rustc_hash::{FxHashMap, FxHashSet};
        let mut buckets: FxHashMap<&str, (&'static [&'static str], Vec<&str>)> = FxHashMap::default();
        for (path, lang, _, sym_count) in &rows {
            if *sym_count == 0 {
                continue;
            }
            let kinds =
                codesight_language_support::distinctive_symbol_kinds_for_language(lang.as_str());
            if kinds.is_empty() {
                continue;
            }
            buckets
                .entry(lang.as_str())
                .or_insert_with(|| (kinds, Vec::new()))
                .1
                .push(path.as_str());
        }
        let mut distinctive_set: FxHashSet<String> =
            FxHashSet::default();
        for (kinds, paths) in buckets.values() {
            let hits = store.files_with_symbol_kinds_batch(paths, kinds)?;
            distinctive_set.extend(hits);
        }
        let mut root: BTreeMap<String, FileTreeBuildNode> = BTreeMap::new();
        for (path, mut lang, size, sym_count) in rows {
            if sym_count > 0
                && let Some(new_lang) = codesight_language_support::reclassify_file_language(
                    lang.as_str(),
                    path.as_str(),
                    distinctive_set.contains(path.as_str()),
                )
            {
                lang = new_lang.to_string();
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
        // ROADMAP P2.7: when the caller gives no `path` prefix, this is an
        // orientation call — cap depth at 2 so the output is crate + top
        // modules, not a deeply-nested wall. Drilling requires an explicit
        // `path=` on the next call.
        let max_depth = if prefix.is_some() {
            usize::MAX
        } else {
            2
        };
        let items = file_tree_to_json_depth(&root, "", max_depth);
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
        let _ = args.max_tokens; // token budget is enforced by per-section caps below
        let (store, timer) = match self.setup_repo_call(args)? {
            Ok(pair) => pair,
            Err(resp) => return Ok(resp),
        };
        // CUTPLAN P2.4: cache the full body keyed by `(source_id,
        // indexed_at)`. The expensive parts (PageRank over the full
        // dependency graph + repository_rollup) are recomputed only on
        // reingest, not per-call. Target: 50 ms warm / 200 ms cold (was
        // 2810 ms cold).
        let indexed_at = store
            .repo_summary()
            .map(|s| s.indexed_at)
            .unwrap_or_default();
        if let Some(cached) = self.cached_repo_outline_body(&args.source_id, &indexed_at) {
            return self.success_response(&timer, cached);
        }
        // ROADMAP P1.3: `repo_outline` is an *orientation* view, not a
        // symbol dump. A cold agent running one call must be able to name
        // the crates, their roles, and the first few files to read. The
        // shape is deliberately small — per-crate rollups, top-N modules
        // by centrality + size, detected entrypoints, language mix.
        // Symbols belong in `file_outline` / `search_catalog`.
        let rows = store.file_tree_rows()?;
        let filtered_rows: Vec<&(String, String, i64, u64)> = rows
            .iter()
            .filter(|(path, _, _, _)| {
                !codesight_language_support::is_outline_noise_basename(path)
            })
            .collect();

        let adj = store.dependency_graph().unwrap_or_default();
        let pagerank = if adj.is_empty() {
            None
        } else {
            Some(codesight_analysis::graph::compute_pagerank(
                &adj, 0.85, 100, 1e-6,
            ))
        };

        // FIXUP T3.1: ask each registered Language whether the directory
        // hosts a real package manifest (Cargo.toml [package], package.json,
        // pyproject.toml, …). Path-prefix grouping was wrong — `tests/` is
        // not a crate, but the old code put it in `workspace.crates`.
        let root = store.root_path().ok();
        let (crates, extra_roots) =
            partition_workspace(&filtered_rows, root.as_deref(), pagerank.as_ref());
        let top_by_centrality = top_modules_by_centrality(pagerank.as_ref(), 10);
        // FIXUP T3.2: the sort key is file size (bytes), not lines of
        // code. Rename the response field to match `top_modules_by_size`
        // which is already the helper's name.
        let top_by_size = top_modules_by_size(&filtered_rows, 10);
        let entrypoints = detect_entrypoints(&filtered_rows);
        let languages: BTreeMap<String, usize> =
            filtered_rows.iter().fold(BTreeMap::new(), |mut acc, r| {
                *acc.entry(r.1.clone()).or_insert(0) += 1;
                acc
            });

        let body = json!({
            "view": "repo_outline",
            "workspace": { "crates": crates },
            "extra_roots": extra_roots,
            "top_modules_by_centrality": top_by_centrality,
            "top_modules_by_size": top_by_size,
            "entrypoints": entrypoints,
            "languages": languages,
        });
        self.store_repo_outline_body(&args.source_id, &indexed_at, body.clone());
        self.success_response(&timer, body)
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
            // Batch the two lookups. Replaces 2N DB round-trips with 2.
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            let mut outlines = store.file_outlines_batch(&refs)?;
            let languages = store.file_languages_batch(&refs)?;
            let mut results = Vec::with_capacity(paths.len());
            for p in paths {
                let symbols = outlines.remove(p.as_str()).unwrap_or_default();
                let lang = languages.get(p.as_str()).cloned().unwrap_or_default();
                let count = symbols.len();
                results.push(json!({
                    "file": p,
                    "language": lang,
                    "items": symbols,
                    "symbol_count": count,
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
        // Distinguish "indexed but empty" from "unknown to the index".
        // Without this probe a misspelled `file_path` returned `ok:true,
        // items:[]` and the agent never realized the path didn't exist.
        let bytes_opt = store.file_bytes(p)?;
        let raw_bytes = bytes_opt.unwrap_or(0);
        let timer = MetaBuilder::new(raw_bytes).with_confidence(Confidence::Safe);
        let symbols = store.file_outline(p)?;
        let lang = store.file_language(p)?.unwrap_or_default();
        let mut warnings: Vec<Value> = Vec::new();
        if bytes_opt.is_none() {
            warnings.push(json!({
                "code": "NOT_INDEXED_PATH",
                "message": format!(
                    "Path `{p}` is not in the code index. Verify the path with `inspect_structure view=file_tree`, or run `ingest_sources path=. incremental=true` if it should be indexed."
                ),
                "detail": { "path": p },
            }));
        }
        self.success_response_with_warnings(
            &timer,
            json!({
                "view": "file_outline",
                "path": p,
                "language": lang,
                "symbol_count": symbols.len(),
                "items": symbols,
            }),
            warnings,
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
        if view.ancestor_count == 0
            && view.descendant_count == 0
            && let Some(msg) =
                codesight_language_support::class_hierarchy_note_for_path(view.class.file.as_str())
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert("note".to_string(), json!(msg));
            obj.insert("supports_hierarchy".to_string(), json!(false));
        }
        self.success_response(&timer, body)
    }
}

/// FIXUP T3.1: per-package rollup + non-package roots for `repo_outline`.
///
/// Returns `(workspace_crates, extra_roots)` where:
/// - `workspace_crates` only contains directories that any registered
///   `Language::package_name_for_dir` recognises (real Cargo crates with
///   `[package]`, `package.json`, `pyproject.toml`, …).
/// - `extra_roots` contains everything else — `tests/`, `scripts/`,
///   `plugins/`, root-level docs/config — classified by a closed `role`
///   vocabulary so callers can branch on it.
///
/// `root` is the repo's absolute path; when `None` (e.g. an in-memory
/// store), package detection is skipped and every file falls into
/// `extra_roots`.
fn partition_workspace(
    rows: &[&(String, String, i64, u64)],
    root: Option<&Path>,
    pagerank: Option<&codesight_analysis::graph::PageRankResult>,
) -> (Vec<Value>, Vec<Value>) {
    let candidate_dirs: BTreeSet<String> = rows
        .iter()
        .filter_map(|(p, _, _, _)| dir_of(p).map(str::to_string))
        .collect();
    let package_roots = match root {
        Some(r) => discover_package_roots(&candidate_dirs, r),
        None => BTreeMap::new(),
    };

    #[derive(Default)]
    struct CrateAcc {
        name: String,
        rel_path: String,
        files: usize,
        symbols: u64,
        size: u64,
        paths: Vec<String>,
    }
    let mut by_crate: BTreeMap<String, CrateAcc> = BTreeMap::new();
    let mut extras: BTreeMap<String, ExtraAcc> = BTreeMap::new();

    for (path, _lang, size, sym_count) in rows {
        let assigned = nearest_package_root(path, &package_roots);
        if let Some(rel) = assigned {
            let name = package_roots
                .get(&rel)
                .cloned()
                .unwrap_or_else(|| rel.clone());
            let entry = by_crate.entry(rel.clone()).or_insert_with(|| CrateAcc {
                name: name.clone(),
                rel_path: rel.clone(),
                ..CrateAcc::default()
            });
            entry.files += 1;
            entry.symbols = entry.symbols.saturating_add(*sym_count);
            entry.size = entry.size.saturating_add((*size).max(0) as u64);
            entry.paths.push(path.clone());
        } else {
            let (key, role) = extra_role_for_path(path);
            let entry = extras.entry(key.clone()).or_insert_with(|| ExtraAcc {
                path: key.clone(),
                role,
                file_count: 0,
                hint: None,
            });
            entry.file_count += 1;
            if entry.hint.is_none() && role == "other" {
                entry.hint = Some(path.clone());
            }
        }
    }

    let mut crates: Vec<_> = by_crate.into_values().collect();
    crates.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.name.cmp(&b.name)));
    let crates_json: Vec<Value> = crates
        .into_iter()
        .map(|c| {
            let central = if let Some(pr) = pagerank {
                let mut ranked: Vec<(&String, f64)> = c
                    .paths
                    .iter()
                    .filter_map(|p| pr.scores.get(p).map(|s| (p, *s)))
                    .collect();
                ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                ranked.iter().take(3).map(|(p, _)| json!(p)).collect()
            } else {
                Vec::new()
            };
            json!({
                "name": c.name,
                "path": c.rel_path,
                "files": c.files,
                "symbols": c.symbols,
                "size_bytes": c.size,
                "central_files": central,
            })
        })
        .collect();

    let mut extras_vec: Vec<_> = extras.into_values().collect();
    extras_vec.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then_with(|| a.path.cmp(&b.path))
    });
    let extras_json: Vec<Value> = extras_vec
        .into_iter()
        .map(|e| {
            let mut obj = json!({
                "path": e.path,
                "role": e.role,
                "file_count": e.file_count,
            });
            if let Some(hint) = e.hint
                && let Value::Object(ref mut m) = obj
            {
                m.insert("hint".to_string(), json!(hint));
            }
            obj
        })
        .collect();

    (crates_json, extras_json)
}

#[derive(Default)]
struct ExtraAcc {
    path: String,
    role: &'static str,
    file_count: usize,
    hint: Option<String>,
}

/// Repo-relative directory of `path`, or `None` for top-level files.
fn dir_of(path: &str) -> Option<&str> {
    path.rfind('/').map(|i| &path[..i])
}

/// Walk every candidate directory and ask each registered language whether
/// it recognises the directory as a package root. Returns a map of
/// `repo_relative_dir → package_name`.
fn discover_package_roots(
    candidate_dirs: &BTreeSet<String>,
    root: &Path,
) -> BTreeMap<String, String> {
    let registry = codesight_language_support::builtin_registry();
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    // Include all ancestor dirs as candidates too — files live N levels
    // below their package root.
    let mut all_dirs: BTreeSet<String> = BTreeSet::new();
    for d in candidate_dirs {
        let mut cur: &str = d.as_str();
        loop {
            all_dirs.insert(cur.to_string());
            match cur.rfind('/') {
                Some(i) => cur = &cur[..i],
                None => break,
            }
        }
    }
    // Also probe the repo root itself.
    all_dirs.insert(String::new());

    for rel in &all_dirs {
        let abs: PathBuf = if rel.is_empty() {
            root.to_path_buf()
        } else {
            root.join(rel)
        };
        if let Some(name) = registry
            .all()
            .find_map(|lang| lang.package_name_for_dir(&abs))
        {
            found.insert(rel.clone(), name);
        }
    }
    found
}

/// Find the longest package-root prefix that contains `file_path`.
fn nearest_package_root(
    file_path: &str,
    roots: &BTreeMap<String, String>,
) -> Option<String> {
    let mut best: Option<&str> = None;
    for root in roots.keys() {
        let matches = if root.is_empty() {
            true
        } else {
            file_path.starts_with(root.as_str())
                && (file_path.len() == root.len()
                    || file_path.as_bytes().get(root.len()) == Some(&b'/'))
        };
        if matches && best.map(|b| root.len() > b.len()).unwrap_or(true) {
            best = Some(root.as_str());
        }
    }
    best.map(str::to_string)
}

/// Classify a non-package file into an `extra_roots` bucket. Returns
/// `(bucket_key, role)` where the role is from the closed vocabulary
/// documented in the FIXUP plan.
fn extra_role_for_path(path: &str) -> (String, &'static str) {
    let top = path.split('/').next().unwrap_or("");
    let role = match top {
        "tests" => "tests",
        "scripts" | "tools" => "scripts",
        "plugins" => "plugins",
        ".cargo" | ".config" | ".github" => "config",
        "docs" | "doc" => "docs",
        "" => "other",
        _ if !path.contains('/') => "other",
        _ => "other",
    };
    let key = if top.is_empty() || !path.contains('/') {
        ".".to_string()
    } else {
        top.to_string()
    };
    (key, role)
}

fn top_modules_by_centrality(
    pagerank: Option<&codesight_analysis::graph::PageRankResult>,
    limit: usize,
) -> Vec<Value> {
    let Some(pr) = pagerank else {
        return Vec::new();
    };
    let mut ranked: Vec<(&String, f64)> = pr
        .scores
        .iter()
        .filter(|(p, _)| !codesight_language_support::is_outline_noise_basename(p.as_str()))
        .map(|(p, s)| (p, *s))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(limit)
        .map(|(p, s)| json!({ "path": p, "score": s }))
        .collect()
}

fn top_modules_by_size(
    rows: &[&(String, String, i64, u64)],
    limit: usize,
) -> Vec<Value> {
    let mut sorted: Vec<&&(String, String, i64, u64)> = rows.iter().collect();
    sorted.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    sorted
        .into_iter()
        .take(limit)
        .map(|(path, lang, size, symbol_count)| {
            json!({
                "path": path,
                "language": lang,
                "size_bytes": size,
                "symbol_count": symbol_count,
            })
        })
        .collect()
}

/// Language-agnostic entrypoint detection. Looks for conventional top-of-
/// crate entry files (`lib.rs`, `main.rs`, `mod.rs` under a crate src root,
/// `__init__.py`, `__main__.py`, `index.ts`, `index.js`). Returned paths
/// are repo-relative and deduplicated.
fn detect_entrypoints(rows: &[&(String, String, i64, u64)]) -> Vec<Value> {
    let mut entries: Vec<Value> = Vec::new();
    for (path, lang, _, _) in rows {
        let role = match path.as_str() {
            _ if path.ends_with("/src/lib.rs") || path == "src/lib.rs" => Some("lib"),
            _ if path.ends_with("/src/main.rs") || path == "src/main.rs" => Some("bin"),
            _ if path.ends_with("/__init__.py") => Some("package_init"),
            _ if path.ends_with("/__main__.py") => Some("package_main"),
            _ if path.ends_with("/index.ts") || path.ends_with("/index.tsx") => Some("ts_index"),
            _ if path.ends_with("/index.js") || path.ends_with("/index.jsx") => Some("js_index"),
            _ => None,
        };
        if let Some(role) = role {
            entries.push(json!({ "path": path, "role": role, "language": lang }));
        }
    }
    entries
}
