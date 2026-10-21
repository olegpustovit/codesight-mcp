//! `search_catalog` — BM25 symbols/files/sections with optional fuzzy + semantic layers.

mod dense;
mod files;
mod fuzzy;
mod intent;
mod plan;
mod response;
mod semantic;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::{search_hit_section, search_hit_symbol};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::agent_v1::warnings;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::{Confidence, MetaBuilder, search_rg_equivalent_bytes};
use crate::service::{CodeService, MAX_QUERY_LEN};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

use dense::{
    materialize_dense_section_items, materialize_dense_symbol_items, merge_dense_ranked_items,
};
// Tier 7.1: post-RRF rerank — referenced via the `dense::` path so the
// caller can read the explicit module qualifier for the rerank rules.
use intent::apply_intent_doc_penalty;
use plan::{
    CodeSideStats, KindFlags, LexicalStats, MetaExtras, SearchCounts, SearchPlan, build_meta_extras,
};
use response::{FormatCtx, format_output};
use semantic::{run_dense_lane, semantic_fallback_warning};

/// File-class prior applied to natural-language search results. Items are
/// partitioned into these buckets and concatenated in order, preserving
/// the ranker's in-bucket sequence. Prod code outranks tests / fixtures
/// at equal rank without introducing a scoring knob the user has to tune.
///
/// `Vendored` sits last — tokenizer dumps, vendored assets, and oversized
/// JSON blobs get pushed past every legitimate test/fixture hit.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FileClass {
    Prod = 0,
    Script = 1,
    Bench = 2,
    Test = 3,
    Fixture = 4,
    Vendored = 5,
}

fn classify_path(path: &str) -> FileClass {
    let has_prefix = |p: &str| path == p || path.split('/').rev().skip(1).any(|part| part == p);
    if codesight_language_support::is_vendored_path(path) {
        return FileClass::Vendored;
    }
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
    let path_class = classify_path(path);
    if path_class != FileClass::Prod {
        return path_class;
    }
    // Inline test detection: a symbol whose qualified name lives inside a
    // `tests` module (`::tests.foo`, `::tests::foo`) is functionally a test
    // even when the file path is prod source.
    let id = item
        .get("ref")
        .and_then(|r| r.get("id").or_else(|| r.get("stable_id")))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.contains("::tests.") || id.contains("::tests::") {
        return FileClass::Test;
    }
    FileClass::Prod
}

/// Merge items that refer to the same symbol / section / file into one row,
/// keeping the max score. Without this, hybrid retrieval can emit duplicate
/// rows for the same `stable_id` — once from the symbol layer and once from
/// the section layer (ROADMAP P1.2).
fn dedupe_by_stable_id(items: &mut Vec<Value>) {
    use rustc_hash::FxHashMap;
    let taken = std::mem::take(items);
    let mut seen: FxHashMap<String, usize> = FxHashMap::default();
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
    let mut buckets: [(FileClass, Vec<Value>); 6] = [
        (FileClass::Prod, Vec::new()),
        (FileClass::Script, Vec::new()),
        (FileClass::Bench, Vec::new()),
        (FileClass::Test, Vec::new()),
        (FileClass::Fixture, Vec::new()),
        (FileClass::Vendored, Vec::new()),
    ];
    for item in taken {
        let class = item_file_class(&item);
        let slot = match class {
            FileClass::Prod => 0,
            FileClass::Script => 1,
            FileClass::Bench => 2,
            FileClass::Test => 3,
            FileClass::Fixture => 4,
            FileClass::Vendored => 5,
        };
        buckets[slot].1.push(item);
    }
    for (_, bucket) in buckets {
        items.extend(bucket);
    }
}

/// Apply the shared vendor/asset deprioritization to a results list.
///
/// Returns an optional `SCOPE_UNREACHABLE_VENDORED`-style warning plus a
/// re-ordered items list where vendored hits are pushed to the tail with a
/// heavy score penalty. Respects the caller's opt-in: when `include_vendored`
/// is true, or when `scope=` explicitly targets an excluded segment, no
/// penalty is applied.
///
/// Oversized JSON blobs are detected via `file_bytes_lookup` — a callback
/// that maps `path -> size_bytes` so we only hit storage once per ranked
/// result set.
fn apply_vendor_penalty<F>(
    items: &mut Vec<Value>,
    include_vendored: bool,
    scope: Option<&str>,
    mut file_bytes_lookup: F,
) where
    F: FnMut(&str) -> Option<u64>,
{
    if include_vendored {
        return;
    }
    if let Some(sc) = scope
        && codesight_language_support::scope_opts_in(sc)
    {
        return;
    }
    // Two-pass: partition non-vendored vs vendored; apply score penalty to
    // vendored rows; append vendored at the tail so a caller asking for
    // limit=25 gets real code in the top slots before the tokenizer vocab.
    let taken = std::mem::take(items);
    let mut keep: Vec<Value> = Vec::with_capacity(taken.len());
    let mut penalized: Vec<Value> = Vec::new();
    for mut item in taken {
        let path = item
            .get("ref")
            .and_then(|r| r.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let size = if path.is_empty() {
            None
        } else {
            file_bytes_lookup(&path)
        };
        if codesight_language_support::is_default_excluded(&path, size) {
            // Heavy score penalty plus tail ordering — a vendored 0.9
            // hit collapses to 0.18, below any legitimate 0.20 prod hit
            // even before the bucketed re-order runs.
            if let Some(obj) = item.as_object_mut() {
                let old = obj.get("score").and_then(Value::as_f64).unwrap_or(0.0);
                obj.insert("score".to_string(), json!((old * 0.2).max(0.0)));
            }
            penalized.push(item);
        } else {
            keep.push(item);
        }
    }
    keep.extend(penalized);
    *items = keep;
}

/// Calibrate the confidence of a keyword/regex search response from relative
/// signals only (BM25 scores aren't comparable across queries — see
/// AGENTS.md §1).
///
/// Returns `Partial` and a non-empty warning list when the result set is
/// suggestive rather than authoritative. Signals:
///
/// - [`OVERBROAD_QUERY`](warnings::OVERBROAD_QUERY): `total_matches > limit × 10`.
/// - [`NO_EXACT_MATCH`](warnings::NO_EXACT_MATCH): no item is `match_type=="exact"`
///   and no item scored ≥ 0.7.
/// - [`AMBIGUOUS_IDENTIFIER`](warnings::AMBIGUOUS_IDENTIFIER): top-3 scores cluster
///   within 0.05 — the ranker can't distinguish the leaders.
fn calibrate_keyword_confidence(
    items: &[Value],
    total_matches: usize,
    limit: usize,
    query: &str,
) -> (Confidence, Vec<Value>) {
    const OVERBROAD_FACTOR: usize = 10;
    const HIGH_SCORE: f64 = 0.7;
    const TOP_K_GAP: f64 = 0.05;

    let mut out = Vec::new();
    let mut downgrade = false;

    if limit > 0 && total_matches > limit.saturating_mul(OVERBROAD_FACTOR) {
        out.push(warnings::warning(
            warnings::OVERBROAD_QUERY,
            format!(
                "{total_matches} matches for '{query}' exceeds {OVERBROAD_FACTOR}× limit ({limit}); narrow with `scope=` or a more specific identifier."
            ),
        ));
        downgrade = true;
    }

    if items.is_empty() {
        return (
            if downgrade {
                Confidence::Partial
            } else {
                Confidence::Safe
            },
            out,
        );
    }

    let any_exact = items
        .iter()
        .any(|i| i.get("match_type").and_then(Value::as_str) == Some("exact"));
    let max_score = items
        .iter()
        .filter_map(|i| i.get("score").and_then(Value::as_f64))
        .fold(0.0_f64, f64::max);
    if !any_exact && max_score < HIGH_SCORE {
        out.push(warnings::warning(
            warnings::NO_EXACT_MATCH,
            format!(
                "no item literally matches '{query}' and the top score is {max_score:.2} (< {HIGH_SCORE:.1}); results are suggestive, not authoritative."
            ),
        ));
        downgrade = true;
    }

    if items.len() >= 3 {
        let mut scores: Vec<f64> = items
            .iter()
            .filter_map(|i| i.get("score").and_then(Value::as_f64))
            .collect();
        if scores.len() >= 3 {
            scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let gap = scores[0] - scores[2];
            if gap < TOP_K_GAP {
                out.push(warnings::warning(
                    warnings::AMBIGUOUS_IDENTIFIER,
                    format!(
                        "top-3 scores cluster within {TOP_K_GAP} ({:.3}, {:.3}, {:.3}) — the ranker can't distinguish them.",
                        scores[0], scores[1], scores[2],
                    ),
                ));
                downgrade = true;
            }
        }
    }

    (
        if downgrade {
            Confidence::Partial
        } else {
            Confidence::Safe
        },
        out,
    )
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
                        "message": "Doc index not present; NL queries run code-only. \
                             Run `ingest_sources path=<repo> domains=docs` once, \
                             then retry with domain=hybrid for hybrid ranking.".to_string(),
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
        let mut regex_bytes_scanned: u64 = 0;
        let mut regex_files_scanned: usize = 0;
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
                    args.per_file_limit,
                )
                .map_err(index_error)?;
            counts.add(result.total_matches, result.files_with_matches);
            more_file_hits = result.more_file_hits;
            regex_bytes_scanned = result.bytes_scanned;
            regex_files_scanned = result.files_scanned;
            items.extend(result.items);
        }
        let mut name_only_applied = false;
        if flags.want_symbol || (!flags.want_file && !flags.want_section) {
            // FIXUP T1.3: `name_only=true` only makes sense in keyword mode
            // (regex routes to file text; NL uses a different lane). Silently
            // ignored otherwise rather than erroring — the meta extra echoes
            // whether the flag actually applied.
            let name_only = args.name_only.unwrap_or(false)
                && args.query_mode.as_deref().unwrap_or("keyword") == "keyword";
            name_only_applied = name_only;
            let mut symbols = self.run_search_ex(
                store,
                args.query.clone(),
                scope.cloned(),
                exclude,
                args.kind.clone(),
                per_kind_cap,
                name_only,
            )?;
            let fuzzy_explicit = args.fuzzy == Some(true);
            // Auto-fuzzy: BM25 produced nothing → try the fuzzy lane so
            // typo'd identifier searches still surface the nearest name.
            let auto_fuzzy = !fuzzy_explicit && symbols.is_empty();
            if fuzzy_explicit && !symbols.is_empty() {
                // Tier 1.2: BM25 returns hits for any token overlap, not
                // just literal-equal names. Only label `exact` when the
                // symbol name matches the query verbatim (ignore case).
                // Promote pure prefixes to `prefix`; everything else is a
                // partial token-overlap hit and gets `partial`.
                let q = args.query.as_str();
                for s in &mut symbols {
                    let label = if s.name.eq_ignore_ascii_case(q) {
                        "exact"
                    } else if s
                        .name
                        .to_ascii_lowercase()
                        .starts_with(&q.to_ascii_lowercase())
                    {
                        "prefix"
                    } else {
                        "partial"
                    };
                    s.match_type = Some(label.to_string());
                }
            }
            if fuzzy_explicit {
                // FIXUP T1.5: always compute the fuzzy lane when the caller
                // explicitly requested `fuzzy=true` — even if BM25 returned
                // zero exact hits.
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
            name_only_applied,
            regex_bytes_scanned,
            regex_files_scanned,
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
        if args.query_mode.as_deref() != Some("regex") {
            dedupe_by_stable_id(&mut items);
        }
        // Vendor deprioritization runs for every query_mode — tokenizer
        // dumps and asset payloads shouldn't outrank real code whether
        // the caller typed `keyword`, `regex`, or `natural_language`.
        // The bucketed directory prior still only runs for NL because it
        // is more aggressive about demoting test files.
        apply_vendor_penalty(
            &mut items,
            args.include_vendored.unwrap_or(false),
            plan.scope.as_deref(),
            |path| {
                code_store
                    .as_ref()
                    .and_then(|s| s.file_bytes(path).ok().flatten())
            },
        );
        // The fuzzy lane needs the same vendor deprioritization — otherwise a
        // typo match in `assets/embedder/tokenizer.json` could outrank the
        // real symbol in `src/`.
        apply_vendor_penalty(
            &mut fuzzy_items,
            args.include_vendored.unwrap_or(false),
            plan.scope.as_deref(),
            |path| {
                code_store
                    .as_ref()
                    .and_then(|s| s.file_bytes(path).ok().flatten())
            },
        );
        // Query-intent penalty: when the user asks a code-shaped question,
        // doc-section hits get a 0.4× multiplier (softened to 0.5× when the
        // best raw doc and code scores are within 0.3) so structural
        // answers outrank prose. Stacks multiplicatively with the vendor
        // penalty (we mutate the same `score` field in place). Skipped
        // when the caller passed `unit_kinds=["doc_section"]` — that's an
        // explicit opt-in to doc-section retrieval, surfaced as `1.0` in
        // meta.
        let (doc_section_penalty, doc_section_rank_demotion) = if plan.doc_section_opt_in {
            (1.0, 0)
        } else {
            let p = apply_intent_doc_penalty(&mut items, plan.query_intent);
            (p.multiplier, p.rank_demotion)
        };
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
                kind.starts_with('h')
                    && kind.len() >= 2
                    && kind[1..].chars().all(|c| c.is_ascii_digit())
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
        // Confidence (Tier 1.1):
        // - `natural_language` and fuzzy/semantic-augmented passes are
        //   inherently heuristic — already `Partial`.
        // - For pure keyword/regex over the authoritative index, downgrade
        //   from `Safe` to `Partial` when calibration signals fire
        //   (NO_EXACT_MATCH / AMBIGUOUS_IDENTIFIER / OVERBROAD_QUERY). BM25
        //   scores aren't comparable across queries, so calibration uses
        //   *relative* signals only.
        let confidence = match plan.query_mode_effective.as_str() {
            "natural_language" => Confidence::Partial,
            _ if stats.fuzzy_applied || semantic_requested => Confidence::Partial,
            _ => {
                let (conf, calib) = calibrate_keyword_confidence(
                    &items,
                    counts.total_matches,
                    plan.lim,
                    &args.query,
                );
                warnings.extend(calib);
                conf
            }
        };
        let timer = MetaBuilder::new(raw_bytes)
            .with_class(crate::service::savings::RetrievalClass::Search)
            .with_confidence(confidence);
        let extra = build_meta_extras(MetaExtras {
            plan: &plan,
            fuzzy_applied: stats.fuzzy_applied,
            semantic_requested,
            name_only_applied: stats.name_only_applied,
            doc_section_penalty,
            doc_section_rank_demotion,
            regex_bytes_scanned: stats.regex_bytes_scanned,
            regex_files_scanned: stats.regex_files_scanned,
        });

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

    #[allow(clippy::too_many_arguments)]
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
        let mut name_only_applied = false;
        let mut regex_bytes_scanned: u64 = 0;
        let mut regex_files_scanned: usize = 0;
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
            name_only_applied = stats.name_only_applied;
            regex_bytes_scanned = stats.regex_bytes_scanned;
            regex_files_scanned = stats.regex_files_scanned;
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
        {
            self.search_doc_side(dstore, args, plan.scope.as_ref(), plan.per_kind_cap, items)?;
        }
        Ok(LexicalStats {
            more_file_hits_exist,
            fuzzy_applied,
            name_only_applied,
            regex_bytes_scanned,
            regex_files_scanned,
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
        // Tier 7.1: deterministic post-RRF rerank for NL queries. Only
        // applied when the caller explicitly chose `mode=nl` — keyword and
        // regex callers asked for a literal match; we don't second-guess
        // their ordering. See `apply_nl_rerank` for the rules and the
        // cross-encoder follow-up note.
        if plan.query_mode_effective == "natural_language" {
            dense::apply_nl_rerank(items, &args.query);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;
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

    fn scored(path: &str, score: f64) -> Value {
        json!({
            "ref": { "path": path, "kind": "file" },
            "snippet": "body",
            "score": score,
        })
    }

    #[test]
    fn apply_vendor_penalty_pushes_vendored_to_tail_and_scales_score() {
        let mut items = vec![
            scored(
                "crates/codesight-retrieval/assets/embedder/tokenizer.json",
                0.9,
            ),
            scored("src/main.rs", 0.4),
            scored("tests/fixtures/sample.cs", 0.85),
            scored("src/lib.rs", 0.3),
        ];
        apply_vendor_penalty(&mut items, false, None, |_| None);
        let order: Vec<(&str, f64)> = items
            .iter()
            .map(|i| {
                (
                    i.pointer("/ref/path").and_then(Value::as_str).unwrap(),
                    i.get("score").and_then(Value::as_f64).unwrap(),
                )
            })
            .collect();
        // Non-vendored retained original order.
        assert_eq!(order[0].0, "src/main.rs");
        assert_eq!(order[1].0, "src/lib.rs");
        // Vendored pushed to the tail, scores scaled down.
        assert!(order[2].0.ends_with("tokenizer.json"));
        assert!(order[3].0.ends_with("sample.cs"));
        assert!(order[2].1 < 0.2);
        assert!(order[3].1 < 0.2);
    }

    #[test]
    fn apply_vendor_penalty_noop_when_include_vendored_true() {
        let mut items = vec![
            scored("tests/fixtures/x.rs", 0.9),
            scored("src/main.rs", 0.4),
        ];
        apply_vendor_penalty(&mut items, true, None, |_| None);
        assert_eq!(
            items[0].pointer("/ref/path").and_then(Value::as_str),
            Some("tests/fixtures/x.rs")
        );
    }

    #[test]
    fn apply_vendor_penalty_noop_when_scope_opts_in() {
        let mut items = vec![
            scored("tests/fixtures/x.rs", 0.9),
            scored("src/main.rs", 0.4),
        ];
        apply_vendor_penalty(&mut items, false, Some("tests/fixtures/"), |_| None);
        assert_eq!(
            items[0].pointer("/ref/path").and_then(Value::as_str),
            Some("tests/fixtures/x.rs")
        );
    }

    #[test]
    fn apply_vendor_penalty_detects_large_json_via_size_lookup() {
        let mut items = vec![
            scored("big.json", 0.9),
            scored("small.json", 0.5),
            scored("src/main.rs", 0.4),
        ];
        apply_vendor_penalty(&mut items, false, None, |path| match path {
            "big.json" => Some(200_000),
            "small.json" => Some(4_096),
            _ => None,
        });
        let paths: Vec<&str> = items
            .iter()
            .map(|i| i.pointer("/ref/path").and_then(Value::as_str).unwrap())
            .collect();
        // big.json is deprioritized, small.json and source stay.
        assert_eq!(paths, vec!["small.json", "src/main.rs", "big.json"]);
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
            [("dense".to_string(), symbol_item("dense"))]
                .into_iter()
                .collect::<FxHashMap<_, _>>(),
            FxHashMap::default(),
        );
        let ids: Vec<&str> = merged
            .iter()
            .filter_map(|item| item.pointer("/ref/id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["lex", "dense"]);
        assert_eq!(merged.last().unwrap()["ref"]["kind"], "file");
    }
}
