//! `search_catalog` — BM25 symbols/files/sections with optional fuzzy + semantic layers.

mod dense;
mod files;
mod fuzzy;
mod plan;
mod response;
mod semantic;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::{search_hit_section, search_hit_symbol};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::{MetaBuilder, search_rg_equivalent_bytes};
use crate::service::{CodeService, MAX_QUERY_LEN};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

use dense::{
    materialize_dense_section_items, materialize_dense_symbol_items, merge_dense_ranked_items,
};
use plan::{
    CodeSideStats, KindFlags, LexicalStats, SearchCounts, SearchPlan, build_meta_extras,
};
use response::{FormatCtx, format_output};
use semantic::{run_dense_lane, semantic_fallback_warning};

/// File-class prior applied to natural-language search results. Items are
/// partitioned into these buckets and concatenated in order, preserving
/// the ranker's in-bucket sequence. Prod code outranks tests / fixtures
/// at equal rank without introducing a scoring knob the user has to tune.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FileClass {
    Prod = 0,
    Script = 1,
    Bench = 2,
    Test = 3,
    Fixture = 4,
}

fn classify_path(path: &str) -> FileClass {
    let has_prefix = |p: &str| {
        path == p || path.starts_with(&format!("{p}/")) || path.contains(&format!("/{p}/"))
    };
    if codesight_language_support::is_test_file(path) || has_prefix("tests") {
        return FileClass::Test;
    }
    if has_prefix("benches") {
        return FileClass::Bench;
    }
    if codesight_language_support::is_test_fixture(path)
        || has_prefix("fixtures")
        || has_prefix("testdata")
        || has_prefix("__fixtures__")
    {
        return FileClass::Fixture;
    }
    if has_prefix("scripts") {
        return FileClass::Script;
    }
    FileClass::Prod
}

fn item_file_class(item: &Value) -> FileClass {
    let path = item
        .get("ref")
        .and_then(|r| r.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    classify_path(path)
}

/// Merge items that refer to the same symbol / section / file into one row,
/// keeping the max score. Without this, hybrid retrieval can emit duplicate
/// rows for the same `stable_id` — once from the symbol layer and once from
/// the section layer (ROADMAP P1.2).
fn dedupe_by_stable_id(items: &mut Vec<Value>) {
    use std::collections::HashMap;
    let taken = std::mem::take(items);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Value> = Vec::with_capacity(taken.len());
    for item in taken {
        let key = item
            .get("ref")
            .and_then(|r| r.get("id").or_else(|| r.get("stable_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let Some(key) = key else {
            out.push(item);
            continue;
        };
        if let Some(&idx) = seen.get(&key) {
            let existing_score = out[idx]
                .get("score")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let new_score = item.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if new_score > existing_score {
                out[idx] = item;
            }
        } else {
            seen.insert(key, out.len());
            out.push(item);
        }
    }
    *items = out;
}

/// Drop items classified as tests / fixtures. Used for NL queries when the
/// caller has not explicitly opted in to `include_tests: true` (ROADMAP P1.2
/// — NL queries asking "how does X work" should surface the production code,
/// not the test asserting it).
fn drop_tests(items: &mut Vec<Value>) {
    items.retain(|item| {
        !matches!(
            item_file_class(item),
            FileClass::Test | FileClass::Fixture | FileClass::Bench
        )
    });
}

fn apply_directory_prior(items: &mut Vec<Value>) {
    // Stable, bucketed re-order. We preserve in-bucket ordering so the
    // underlying RRF fusion still decides *which* test file (if any)
    // outranks another test file — we only prevent them from occupying
    // the top slots when equally ranked prod code exists.
    let taken = std::mem::take(items);
    let mut buckets: [(FileClass, Vec<Value>); 5] = [
        (FileClass::Prod, Vec::new()),
        (FileClass::Script, Vec::new()),
        (FileClass::Bench, Vec::new()),
        (FileClass::Test, Vec::new()),
        (FileClass::Fixture, Vec::new()),
    ];
    for item in taken {
        let class = item_file_class(&item);
        let slot = match class {
            FileClass::Prod => 0,
            FileClass::Script => 1,
            FileClass::Bench => 2,
            FileClass::Test => 3,
            FileClass::Fixture => 4,
        };
        buckets[slot].1.push(item);
    }
    for (_, bucket) in buckets {
        items.extend(bucket);
    }
}

impl CodeService {
    fn resolve_stores(
        &self,
        args: &SearchCatalogArgs,
        domain: &str,
        flags: &KindFlags,
        warnings: &mut Vec<Value>,
    ) -> Result<(Option<IndexStore>, Option<DocStore>), ToolError> {
        let need_code = domain != "docs" && (flags.want_symbol || flags.want_file);
        let need_doc =
            domain != "code" && (flags.want_section || domain == "docs" || domain == "hybrid");

        let code_store = if need_code {
            match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" => {
                    warnings.push(json!({
                        "code": "CODE_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let doc_store = if need_doc {
            match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                // ROADMAP P2.2: replace the bare DOC_INDEX_MISSING noise
                // with a concrete PROMOTE_TO_HYBRID hint carrying the next
                // tool call (`ingest_sources domains=docs`). Agents act on
                // hints; they ignore generic warnings.
                Err(e) if domain == "hybrid" && code_store.is_some() => {
                    warnings.push(json!({
                        "code": "PROMOTE_TO_HYBRID",
                        "message": format!(
                            "Doc index not present; NL queries run code-only. \
                             Run `ingest_sources path=<repo> domains=docs` once, \
                             then retry with domain=hybrid for hybrid ranking."
                        ),
                        "detail": {
                            "missing": "docs",
                            "source_id": args.source_id,
                            "reason": e.message,
                        },
                        "suggested_tool": {
                            "tool": "ingest_sources",
                            "args": { "source_id": args.source_id, "domains": "docs" },
                        },
                    }));
                    None
                }
                Err(e) if domain == "docs" || domain == "hybrid" => return Err(e),
                Err(_) => None,
            }
        } else {
            None
        };

        if code_store.is_none() && doc_store.is_none() {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "NOT_INDEXED",
                message: format!(
                    "Source '{}' has no usable index for this search.",
                    args.source_id
                ),
                detail: json!({ "source_id": args.source_id, "domain": domain }),
            });
        }
        Ok((code_store, doc_store))
    }

    #[allow(clippy::too_many_arguments)]
    fn search_code_side(
        &self,
        store: &IndexStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        flags: &KindFlags,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
        fuzzy_items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<CodeSideStats, ToolError> {
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
        let want_body = args.include_body.unwrap_or(false);
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let mut more_file_hits = false;
        let mut fuzzy_applied = false;
        if flags.want_file {
            let qmode = args.query_mode.as_deref().unwrap_or("keyword");
            let result = self
                .search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.cloned(),
                    exclude,
                    per_kind_cap,
                    qmode,
                    ctx_lines,
                )
                .map_err(index_error)?;
            counts.add(result.total_matches, result.files_with_matches);
            more_file_hits = result.more_file_hits;
            items.extend(result.items);
        }
        if flags.want_symbol || (!flags.want_file && !flags.want_section) {
            let mut symbols = self.run_search(
                store,
                args.query.clone(),
                scope.cloned(),
                exclude,
                args.kind.clone(),
                per_kind_cap,
            )?;
            let fuzzy_explicit = args.fuzzy == Some(true);
            let auto_fuzzy =
                !fuzzy_explicit && symbols.is_empty() && args.fuzzy_threshold.is_some();
            if fuzzy_explicit && !symbols.is_empty() {
                for s in &mut symbols {
                    s.match_type = Some("exact".to_string());
                }
            }
            if fuzzy_explicit && !symbols.is_empty() {
                // ROADMAP P1.4: two-lane response. Exact BM25 hits stay in
                // `items`, the fuzzy lane lives in `fuzzy_items` and is
                // computed independently against trigram+Levenshtein.
                let lane = fuzzy::compute_fuzzy_lane(
                    store,
                    args,
                    scope.map(String::as_str),
                    exclude,
                    per_kind_cap,
                    &symbols,
                );
                fuzzy_applied = !lane.is_empty();
                for s in lane {
                    let body = if want_body {
                        self.symbol_body_with_context(store, &s.id, ctx_lines)
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                    fuzzy_items.push(search_hit_symbol(&args.source_id, &s, body.as_deref()));
                }
            } else if auto_fuzzy {
                // Zero BM25 hits + explicit threshold → promote fuzzy rows
                // into the primary `items` lane (there is no exact lane to
                // displace). Matches today's "fuzzy augmentation" behaviour.
                fuzzy_applied = fuzzy::augment_with_fuzzy(
                    store,
                    args,
                    scope.map(String::as_str),
                    exclude,
                    per_kind_cap,
                    auto_fuzzy,
                    &mut symbols,
                ) > 0;
            }
            for s in symbols {
                let body = if want_body {
                    self.symbol_body_with_context(store, &s.id, ctx_lines)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                items.push(search_hit_symbol(&args.source_id, &s, body.as_deref()));
            }
        }
        Ok(CodeSideStats {
            more_file_hits,
            fuzzy_applied,
        })
    }

    fn search_doc_side(
        &self,
        store: &DocStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
    ) -> Result<(), ToolError> {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let want_body = args.include_body.unwrap_or(false);
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
        let trimmed = args.query.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let effective_query = if trimmed.len() > MAX_QUERY_LEN {
            &trimmed[..MAX_QUERY_LEN]
        } else {
            trimmed
        };
        let sections = store
            .search_sections(
                effective_query,
                scope.map(String::as_str),
                exclude,
                per_kind_cap,
            )
            .map_err(index_error)?;
        for sec in sections {
            let body = if want_body {
                self.section_body_with_context(store, &sec.id, ctx_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            items.push(search_hit_section(&args.source_id, &sec, body.as_deref()));
        }
        Ok(())
    }

    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let mut args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let plan = match SearchPlan::build(&mut args) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let mut warnings = Vec::new();
        let (code_store, doc_store) =
            match self.resolve_stores(&args, &plan.domain, &plan.flags, &mut warnings) {
                Ok(pair) => pair,
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            };

        let mut items: Vec<Value> = Vec::new();
        // ROADMAP P1.4: when `fuzzy=true` is explicit, the fuzzy lane is
        // populated alongside (not inside) the exact `items` lane.
        let mut fuzzy_items: Vec<Value> = Vec::new();
        let mut counts = SearchCounts::default();
        self.run_count_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut counts,
        );

        let stats = match self.run_lexical_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut fuzzy_items,
            &mut counts,
        ) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        let semantic_requested = match self.run_semantic_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut warnings,
        ) {
            Ok(b) => b,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        // NL-only directory prior: partition items by file class so
        // production `src/` code outranks keyword-dense test / fixture /
        // bench files with equal RRF rank. Stable within each bucket so
        // the ranker's ordering wins inside a single class. Also drop
        // duplicate rows produced by the hybrid fusion when the symbol and
        // section layers both find the same stable_id.
        dedupe_by_stable_id(&mut items);
        if plan.query_mode_effective == "natural_language" {
            if !args.include_tests.unwrap_or(false) {
                drop_tests(&mut items);
            }
            apply_directory_prior(&mut items);
        }

        // ROADMAP P2.1: when a hybrid search returns only doc-section hits
        // (no code symbols or code file-body matches), flag that the symbol
        // probably doesn't exist in the codebase. Section hits have ids
        // shaped `<file>::<heading>#h<level>`; code hits either carry a
        // non-doc file extension or a `#<symbol-kind>` suffix.
        if plan.domain == "hybrid" && !items.is_empty() {
            let is_doc_section = |item: &Value| -> bool {
                let id = item
                    .pointer("/ref/id")
                    .and_then(Value::as_str)
                    .or_else(|| item.pointer("/ref/stable_id").and_then(Value::as_str))
                    .unwrap_or("");
                // Doc sections embed `#h<N>` anchor kind in the id. Symbol
                // ids end in `#function`, `#class`, … or `#file`.
                let kind = id.rsplit('#').next().unwrap_or("");
                kind.starts_with('h') && kind.len() >= 2 && kind[1..].chars().all(|c| c.is_ascii_digit())
            };
            let has_code = items.iter().any(|item| !is_doc_section(item));
            let has_docs = items.iter().any(is_doc_section);
            if has_docs && !has_code {
                warnings.push(json!({
                    "code": "SYMBOL_NOT_FOUND_IN_CODE",
                    "message": format!(
                        "all {} hits for '{}' are in documentation; the symbol may have been refactored away or never existed in code.",
                        items.len(),
                        args.query,
                    ),
                }));
            }
        }

        // ROADMAP P1.1: baseline = `rg -n` output size for the returned
        // hit set (path:line:snippet per row), capped intrinsically at the
        // caller's `limit`. Not the sum of file sizes of every matched file.
        let raw_bytes = search_rg_equivalent_bytes(&items);
        let timer = MetaBuilder::new(raw_bytes)
            .with_class(crate::service::savings::RetrievalClass::Search);
        let extra = build_meta_extras(&plan, stats.fuzzy_applied, semantic_requested);

        let fmt_ctx = FormatCtx {
            output_mode: &plan.output_mode,
            query: &args.query,
            query_mode_effective: &plan.query_mode_effective,
            offset: plan.offset,
            lim: plan.lim,
            max_tokens: args.max_tokens,
            more_file_hits_exist: stats.more_file_hits_exist,
            counts,
            fuzzy_items,
        };
        format_output(self, &fmt_ctx, items, &timer, warnings, extra)
    }

    fn run_count_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        counts: &mut SearchCounts,
    ) {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        if let Some(store) = code_store
            && (plan.flags.want_symbol || (!plan.flags.want_file && !plan.flags.want_section))
            && let Ok((m, f)) = store.count_symbols(
                &args.query,
                args.kind.as_deref(),
                plan.scope.as_deref(),
                exclude,
            )
        {
            counts.add(m, f);
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
            && let Ok((m, f)) = dstore.count_sections(&args.query, plan.scope.as_deref(), exclude)
        {
            counts.add(m, f);
        }
    }

    fn run_lexical_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        fuzzy_items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<LexicalStats, ToolError> {
        let mut more_file_hits_exist = false;
        let mut fuzzy_applied = false;
        if let Some(store) = code_store {
            let stats = self.search_code_side(
                store,
                args,
                plan.scope.as_ref(),
                &plan.flags,
                plan.per_kind_cap,
                items,
                fuzzy_items,
                counts,
            )?;
            more_file_hits_exist = stats.more_file_hits;
            fuzzy_applied = stats.fuzzy_applied;
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
        {
            self.search_doc_side(dstore, args, plan.scope.as_ref(), plan.per_kind_cap, items)?;
        }
        Ok(LexicalStats {
            more_file_hits_exist,
            fuzzy_applied,
        })
    }

    fn run_semantic_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        warnings: &mut Vec<Value>,
    ) -> Result<bool, ToolError> {
        let semantic_requested =
            args.semantic.unwrap_or(false) || plan.query_mode_effective == "natural_language";
        if !semantic_requested {
            return Ok(false);
        }
        if !(plan.flags.want_symbol || plan.flags.want_section) {
            warnings.push(semantic_fallback_warning(
                "files_only",
                "request only targets lexical file hits".to_string(),
            ));
            return Ok(true);
        }
        let dense = run_dense_lane(self, &args.query, code_store, doc_store, plan.lim);
        if let Some(w) = dense.warning {
            warnings.push(w);
            return Ok(true);
        }
        let include_body = args.include_body.unwrap_or(false);
        let context_lines = args.context_lines.unwrap_or(0).min(50);
        let dense_symbol_items = materialize_dense_symbol_items(
            self,
            &args.source_id,
            code_store,
            &dense.symbol_hits,
            include_body,
            context_lines,
        )?;
        let dense_section_items = materialize_dense_section_items(
            self,
            &args.source_id,
            doc_store,
            &dense.section_hits,
            include_body,
            context_lines,
        )?;
        let taken = std::mem::take(items);
        *items = merge_dense_ranked_items(
            taken,
            &dense.symbol_hits,
            &dense.section_hits,
            dense_symbol_items,
            dense_section_items,
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic::DenseRankedHit;

    fn symbol_item(id: &str) -> Value {
        json!({
            "ref": { "id": id, "kind": "symbol" },
            "snippet": "sig",
        })
    }

    fn file_item(path: &str) -> Value {
        json!({
            "ref": { "path": path, "kind": "file" },
            "snippet": "body",
        })
    }

    #[test]
    fn dense_only_hits_are_materialized_and_files_are_appended() {
        let merged = merge_dense_ranked_items(
            vec![file_item("src/main.rs"), symbol_item("lex")],
            &[
                DenseRankedHit {
                    id: "dense".to_string(),
                    distance: 0.1,
                },
                DenseRankedHit {
                    id: "lex".to_string(),
                    distance: 0.2,
                },
            ],
            &[],
            std::collections::HashMap::from([("dense".to_string(), symbol_item("dense"))]),
            std::collections::HashMap::new(),
        );
        let ids: Vec<&str> = merged
            .iter()
            .filter_map(|item| item.pointer("/ref/id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["lex", "dense"]);
        assert_eq!(merged.last().unwrap()["ref"]["kind"], "file");
    }
}
