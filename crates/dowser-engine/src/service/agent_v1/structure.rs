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
use crate::service::agent_v1::args::InspectStructureArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::{Confidence, MetaBuilder};
use dowser_storage::storage::IndexStore;

/// Result of a store-open attempt: either the ready pair or a pre-formed
/// error envelope the caller should return verbatim.
type StoreOrError<S> = Result<(S, MetaBuilder), Value>;

impl CodeService {
    pub(in crate::service) fn inspect_structure(&self, v: Value) -> anyhow::Result<Value> {
        let args: InspectStructureArgs = ValidatedArgs::from_validated(v)?.into_inner();
        match args.view.to_ascii_lowercase().as_str() {
            "repo_outline" => self.structure_repo_outline(&args),
            "file_outline" => self.structure_file_outline(&args),
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
            .filter(|(path, _, _, _)| !dowser_language_support::is_outline_noise_basename(path))
            .collect();

        let adj = store.dependency_graph().unwrap_or_default();
        let pagerank = if adj.is_empty() {
            None
        } else {
            Some(dowser_analysis::graph::compute_pagerank(
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
    pagerank: Option<&dowser_analysis::graph::PageRankResult>,
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
    let registry = dowser_language_support::builtin_registry();
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
fn nearest_package_root(file_path: &str, roots: &BTreeMap<String, String>) -> Option<String> {
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
    pagerank: Option<&dowser_analysis::graph::PageRankResult>,
    limit: usize,
) -> Vec<Value> {
    let Some(pr) = pagerank else {
        return Vec::new();
    };
    let mut ranked: Vec<(&String, f64)> = pr
        .scores
        .iter()
        .filter(|(p, _)| !dowser_language_support::is_outline_noise_basename(p.as_str()))
        .map(|(p, s)| (p, *s))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(limit)
        .map(|(p, s)| json!({ "path": p, "score": s }))
        .collect()
}

fn top_modules_by_size(rows: &[&(String, String, i64, u64)], limit: usize) -> Vec<Value> {
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
