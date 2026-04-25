use rustc_hash::FxHashMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use codesight_analysis::graph::CallEdge;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::ContentRef;
use crate::service::agent_v1::util::resolve_symbol_id;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use codesight_core::symbols::SymbolKind;
use codesight_language_support::imports_orchestration::context_bundle_import_lines;
use codesight_storage::storage::IndexStore;

/// Process-wide cache of `build_call_edges` results keyed on
/// `(canonical_db_path, index_version)`. The build itself is O(N×idents)
/// over all 5,500+ symbol bodies and produces ~110k edges on this repo;
/// caching it shaves the remaining ~265ms off warm `assemble_context`
/// `context_bundle` calls. Entries from older index versions are evicted
/// on the next write (we drop any entry whose path matches but version
/// differs at insert time).
type CallEdgeKey = (PathBuf, u32);
type CallEdgeCache = Mutex<FxHashMap<CallEdgeKey, Arc<Vec<CallEdge>>>>;

fn call_edges_cache() -> &'static CallEdgeCache {
    static CACHE: OnceLock<CallEdgeCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(FxHashMap::default()))
}

fn cached_call_edges(
    store: &IndexStore,
) -> anyhow::Result<Arc<Vec<CallEdge>>> {
    let version = store.current_index_version().unwrap_or(0);
    let key = (store.db_path().to_path_buf(), version);
    if let Ok(map) = call_edges_cache().lock()
        && let Some(existing) = map.get(&key)
    {
        return Ok(existing.clone());
    }
    let symbols = store.all_symbols_with_source_cached()?;
    let edges = codesight_analysis::graph::build_call_edges(symbols.as_slice());
    let arc = Arc::new(edges);
    if let Ok(mut map) = call_edges_cache().lock() {
        let path = store.db_path().to_path_buf();
        map.retain(|(p, _), _| p != &path);
        map.insert(key, arc.clone());
    }
    Ok(arc)
}

#[derive(Clone)]
pub(super) struct BundleRow {
    pub sym_id: String,
    pub name: String,
    pub signature: String,
    pub kind: SymbolKind,
    pub file: String,
    pub imports: Vec<String>,
    pub source: String,
}

macro_rules! try_store {
    ($self:expr, $timer:expr, $expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return Ok(Err($self.error_response($timer, index_error(e))?)),
        }
    };
}

impl CodeService {
    pub(super) fn gather_bundle_rows(
        &self,
        store: &IndexStore,
        sym_refs: &[&ContentRef],
        timer: &MetaBuilder,
    ) -> anyhow::Result<Result<Vec<BundleRow>, Value>> {
        // Pass 1 — resolve each ref to an id up front. `resolve_symbol_id`
        // may touch the DB (path+line fallback) but at most once per ref
        // regardless of how we batch the rest.
        let mut ids: Vec<String> = Vec::with_capacity(sym_refs.len());
        for pref in sym_refs {
            let id = match resolve_symbol_id(
                store,
                pref.stable_id.as_deref(),
                pref.id.as_deref(),
                pref.path.as_deref(),
                pref.start_line,
                pref.symbol_name.as_deref(),
            ) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    let message = match (pref.symbol_name.as_deref(), pref.path.as_deref()) {
                        (Some(name), Some(path)) => format!(
                            "no symbol named '{name}' in {path} — try inspect_structure view=file_outline file_path=\"{path}\""
                        ),
                        (Some(name), None) => format!(
                            "symbol name '{name}' is ambiguous or not indexed — pass path + symbol_name or a stable_id"
                        ),
                        _ => "symbol ref needs id/stable_id, path + start_line, or path + symbol_name.".to_string(),
                    };
                    return Ok(Err(self.error_response(
                        timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message,
                            detail: json!({
                                "symbol_name": pref.symbol_name,
                                "path": pref.path,
                            }),
                        },
                    )?));
                }
                Err(e) => return Ok(Err(self.error_response(timer, index_error(e))?)),
            };
            ids.push(id);
        }

        // Pass 2 — batch-fetch everything symbol-keyed in one DB trip each.
        // Replaces 4 single-key DB calls per ref (symbol_record + names +
        // signature + file_language) plus one `root_path()` per iteration.
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let enriched = try_store!(self, timer, store.symbol_enriched_batch(&id_refs));
        let records = try_store!(self, timer, store.symbol_records_batch(&id_refs));
        let root = try_store!(self, timer, store.root_path());

        // Pass 3 — assemble each row. File disk reads are inherently
        // per-ref (to materialize imports) but dedup across refs that
        // share the same file so large bundles don't re-read.
        let mut rows = Vec::with_capacity(sym_refs.len());
        let mut file_cache: FxHashMap<String, String> =
            FxHashMap::default();
        for sym_id in ids {
            let Some(record) = records.get(&sym_id) else {
                return Ok(Err(self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found in index."),
                        detail: json!({ "id": sym_id }),
                    },
                )?));
            };
            let meta = enriched.get(&sym_id);
            // Reuse the shared SymbolIdRef leaf extraction instead of
            // hand-parsing `::` + `#` at each call site.
            let name = meta.map(|m| m.name.clone()).unwrap_or_else(|| {
                codesight_core::SymbolIdRef::new(&sym_id)
                    .leaf_name()
                    .to_string()
            });
            let signature = meta.map(|m| m.signature.clone()).unwrap_or_default();

            let file_content = if let Some(cached) = file_cache.get(&record.file_path) {
                cached.clone()
            } else {
                let file_abs = root.join(&record.file_path);
                let content = match fs::read_to_string(&file_abs) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(Err(self.error_response(
                            timer,
                            ToolError {
                                category: ToolErrorCategory::NotFound,
                                code: "NOT_FOUND",
                                message: format!(
                                    "File '{}' is missing from disk.",
                                    record.file_path
                                ),
                                detail: json!({ "file": record.file_path }),
                            },
                        )?));
                    }
                };
                file_cache.insert(record.file_path.clone(), content.clone());
                content
            };
            let imports = context_bundle_import_lines(&file_content, &record.file_path);

            rows.push(BundleRow {
                sym_id: record.id.clone(),
                name,
                signature,
                kind: record.kind,
                file: record.file_path.clone(),
                imports,
                source: record.stored_source.clone(),
            });
        }
        Ok(Ok(rows))
    }

    /// Expand a `context_bundle` by `depth` (ROADMAP P1.5).
    ///
    /// * depth 0 — returns `base` unchanged.
    /// * depth ≥ 1 — pulls in type/identifier references found in the
    ///   signatures and bodies of the base rows, resolved to indexed symbols
    ///   via globally-unique name lookup. Skips unresolvable identifiers and
    ///   anything already in the bundle.
    /// * depth ≥ 2 — adds a representative caller + callee per base row from
    ///   the text-heuristic call graph (same engine `inspect_graph graph=calls`
    ///   uses). One each, keeping the bundle small enough for agents.
    ///
    /// `include_tests` (default `true`) filters out rows whose `file` path
    /// looks like a test fixture — controlled by
    /// `codesight_language_support::is_test_file`.
    pub(super) fn expand_bundle_by_depth(
        &self,
        store: &IndexStore,
        base: Vec<BundleRow>,
        depth: u32,
        include_tests: bool,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Result<Vec<BundleRow>, Value>> {
        if depth == 0 || base.is_empty() {
            return Ok(Ok(base));
        }
        let total_start = std::time::Instant::now();
        let mut seen: BTreeSet<String> = base.iter().map(|r| r.sym_id.clone()).collect();
        let mut extras: Vec<BundleRow> = Vec::new();

        // Depth 1: tokens that look like type/symbol names in the sigs & bodies.
        let type_name_candidates = collect_type_name_candidates(&base);
        for name in type_name_candidates {
            let Ok(Some(sid)) = store.symbol_id_by_unique_name(&name) else {
                continue;
            };
            if seen.insert(sid.clone())
                && let Some(row) = self.row_for_symbol_id(store, &sid)
            {
                extras.push(row);
            }
        }

        // Depth 2: one caller + one callee per base row, walking the cached
        // call-edge set (process-global, keyed on `(db_path, index_version)`
        // — see `cached_call_edges` above). Without the cache, building the
        // ~110k-edge call graph re-runs on every dispatch, costing ~265ms.
        if depth >= 2 {
            let edges = try_store!(self, timer, cached_call_edges(store));
            for row in &base {
                let callers =
                    codesight_analysis::graph::find_callers(&row.sym_id, &edges, 1);
                if let Some((id, _, _, _, _, _)) = callers.first()
                    && seen.insert(id.clone())
                    && let Some(extra) = self.row_for_symbol_id(store, id)
                {
                    extras.push(extra);
                }
                let callees =
                    codesight_analysis::graph::find_callees(&row.sym_id, &edges, 1);
                if let Some((id, _, _, _, _, _)) = callees.first()
                    && seen.insert(id.clone())
                    && let Some(extra) = self.row_for_symbol_id(store, id)
                {
                    extras.push(extra);
                }
            }
        }

        if !include_tests {
            extras.retain(|r| !codesight_language_support::is_test_file(&r.file));
        }

        // Greedy centrality pack: PageRank scores are cached on the store
        // (process-global, version-keyed), so repeated bundles within an
        // index generation skip the ~1.6s petgraph 50-iter compute.
        let pr_scores = store
            .pagerank_scores_cached_with(|adj| {
                codesight_analysis::graph::compute_pagerank(adj, 0.85, 50, 1e-5).scores
            })
            .ok();
        if let Some(scores) = pr_scores {
            extras.sort_by(|a, b| {
                let sa = scores.get(&a.file).copied().unwrap_or(0.0);
                let sb = scores.get(&b.file).copied().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
        }

        let total_ms = total_start.elapsed().as_millis();
        // Kept at debug level so production logs stay quiet but
        // `RUST_LOG=codesight::bundle_timing=debug` surfaces per-call
        // timings if a regression returns.
        tracing::debug!(
            target: "codesight::bundle_timing",
            total_ms,
            extras_len = extras.len(),
            "expand_bundle_by_depth"
        );

        let mut out = base;
        out.extend(extras);
        Ok(Ok(out))
    }

    /// Build a `BundleRow` from a single resolved stable id. Returns `None`
    /// when the symbol has no indexed metadata or its file is missing from
    /// disk — used by the depth-expansion pass where a missing neighbour is
    /// fine (just skip it).
    fn row_for_symbol_id(&self, store: &IndexStore, sym_id: &str) -> Option<BundleRow> {
        let record = store.symbol_record(sym_id).ok().flatten()?;
        let enriched = store
            .symbol_enriched_batch(&[sym_id])
            .ok()
            .and_then(|mut m| m.remove(sym_id));
        let name = enriched
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| {
                codesight_core::SymbolIdRef::new(sym_id)
                    .leaf_name()
                    .to_string()
            });
        let signature = enriched
            .as_ref()
            .map(|m| m.signature.clone())
            .unwrap_or_default();
        let root = store.root_path().ok()?;
        let file_abs = root.join(&record.file_path);
        let file_content = fs::read_to_string(&file_abs).ok()?;
        let imports = context_bundle_import_lines(&file_content, &record.file_path);
        Some(BundleRow {
            sym_id: record.id.clone(),
            name,
            signature,
            kind: record.kind,
            file: record.file_path.clone(),
            imports,
            source: record.stored_source.clone(),
        })
    }
}

/// Rough heuristic: identifiers in signatures/bodies that *look* like type
/// names — `PascalCase` tokens. This avoids the cost of re-parsing the
/// source while still picking up the ~90% case (types constructed or
/// returned by the bundled symbol). Known-noise / overly-generic words
/// get filtered so tiny bundles don't balloon on `Option`, `Result`, `Vec`.
fn collect_type_name_candidates(rows: &[BundleRow]) -> BTreeSet<String> {
    const IGNORE: &[&str] = &[
        "Option", "Result", "Vec", "String", "FxHashMap", "FxHashSet", "BTreeMap",
        "BTreeSet", "Box", "Arc", "Rc", "Cell", "RefCell", "Mutex", "Self",
        "None", "Some", "Ok", "Err",
    ];
    let mut out: BTreeSet<String> = BTreeSet::new();
    for r in rows {
        for haystack in [r.signature.as_str(), r.source.as_str()] {
            for token in haystack.split(|c: char| !c.is_alphanumeric() && c != '_') {
                let head = token.chars().next();
                let is_camel = head.map(|c| c.is_ascii_uppercase()).unwrap_or(false)
                    && token.chars().skip(1).any(|c| c.is_ascii_lowercase())
                    && token.len() >= 3;
                if is_camel && !IGNORE.contains(&token) {
                    out.insert(token.to_string());
                }
            }
        }
    }
    out
}
