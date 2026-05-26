mod hotspots;
mod repo_health;
mod test_gaps;

use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};
use codesight_analysis::{
    agent_readiness::{AgentReadinessInputs, compute_agent_readiness_for_scope},
    dead_code,
    graph::find_cycles,
    quality_signal,
    thresholds::current_thresholds,
};

/// In-process memoisation backing `evaluate_quality` analyses whose
/// recomputation is dominated by index reads.
///
/// Invalidation is generational: every entry stamps the `catalog_generation`
/// it was computed under; a fresh ingest bumps the counter and stale entries
/// drop on lookup. The TTL is the secondary safety net for file mutations
/// that bypass the watcher (e.g. external `git checkout`) — agents see at
/// most `ttl` seconds of staleness before recompute.
struct GenerationalCacheEntry {
    generation: u64,
    cached_at: Instant,
    payload: Value,
}

struct GenerationalCache<K: Eq + Hash + Clone> {
    inner: Mutex<FxHashMap<K, GenerationalCacheEntry>>,
    ttl: Duration,
    cap: usize,
}

impl<K: Eq + Hash + Clone> GenerationalCache<K> {
    fn new(ttl: Duration, cap: usize) -> Self {
        Self {
            inner: Mutex::new(FxHashMap::default()),
            ttl,
            cap,
        }
    }

    fn lookup(&self, key: &K, current_generation: u64) -> Option<Value> {
        let cache = self.inner.lock().ok()?;
        let entry = cache.get(key)?;
        if entry.generation != current_generation || entry.cached_at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.payload.clone())
    }

    fn store(&self, key: K, generation: u64, payload: Value) {
        let Ok(mut cache) = self.inner.lock() else {
            return;
        };
        if cache.len() >= self.cap
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, e)| e.cached_at)
                .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            GenerationalCacheEntry {
                generation,
                cached_at: Instant::now(),
                payload,
            },
        );
    }
}

/// `dead_code` reachability + string-dispatch detection over every file body
/// — ~8s cold on this repo.
const DEAD_CODE_TTL: Duration = Duration::from_secs(30);
const QUALITY_CACHE_MAX: usize = 32;

/// `quality_signal` governance walk: cyclomatic complexity over every fn,
/// dead-code redundancy contribution, blake3 over each body for dup
/// detection — ~1.3s cold on this repo.
const QUALITY_SIGNAL_TTL: Duration = Duration::from_secs(30);

#[derive(Eq, PartialEq, Hash, Clone)]
struct DeadCodeCacheKey {
    source_id: String,
    granularity: String,
    min_confidence_bits: u64,
    include_tests: bool,
    include_vendored: bool,
    paranoid: bool,
    top_n: usize,
    entry_patterns: Vec<String>,
    max_tokens: Option<u32>,
}

#[derive(Eq, PartialEq, Hash, Clone)]
struct QualitySignalCacheKey {
    source_id: String,
    scope: Option<String>,
}

fn dead_code_cache() -> &'static GenerationalCache<DeadCodeCacheKey> {
    static CACHE: OnceLock<GenerationalCache<DeadCodeCacheKey>> = OnceLock::new();
    CACHE.get_or_init(|| GenerationalCache::new(DEAD_CODE_TTL, QUALITY_CACHE_MAX))
}

/// Tier 5.1: query the dead-code cache without running a fresh analysis.
/// `repo_health` uses this so a cold cache returns the rest of the summary
/// in < 1 s with the dead-code subsection marked `pending`, instead of
/// blocking ~750 ms on a full reachability walk.
///
/// Looks up the cache key that matches the *default* `health analysis=dead_code`
/// invocation (`granularity=file`, `min_confidence=0.5`, `include_tests=false`).
/// Callers who want a fresh dead-code count should run that analysis directly.
pub(super) fn cached_default_dead_code_count(svc: &CodeService, source_id: &str) -> Option<u64> {
    let key = DeadCodeCacheKey {
        source_id: source_id.to_string(),
        granularity: "file".to_string(),
        min_confidence_bits: 0.5_f64.to_bits(),
        include_tests: false,
        include_vendored: false,
        paranoid: false,
        top_n: 25,
        entry_patterns: Vec::new(),
        max_tokens: None,
    };
    let generation = svc.catalog_generation();
    dead_code_cache()
        .lookup(&key, generation)
        .and_then(|d| d.get("dead_file_count").and_then(|v| v.as_u64()))
}

fn quality_signal_cache() -> &'static GenerationalCache<QualitySignalCacheKey> {
    static CACHE: OnceLock<GenerationalCache<QualitySignalCacheKey>> = OnceLock::new();
    CACHE.get_or_init(|| GenerationalCache::new(QUALITY_SIGNAL_TTL, QUALITY_CACHE_MAX))
}

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);
        // ROADMAP P3.6: wall-clock budget. Wrap the inner dispatch so every
        // analysis handler plays by the same rules: when `max_wait_ms` is
        // set and elapsed time exceeds it, either error out
        // (`DEADLINE_EXCEEDED`) or return the finished payload stamped with
        // a `BUDGET_EXCEEDED` warning when `partial_ok=true`. Finer-grained
        // cancellation inside the heavier handlers is follow-up work — this
        // contract gives callers the bound they can act on now.
        let start = std::time::Instant::now();
        let inner = match a.as_str() {
            "repo_health" => self.quality_repo_health(&args, &timer),
            "dead_code" => self.quality_dead_code(&args, &timer),
            "quality_signal" => self.quality_signal_handler(&args, &timer),
            "hotspots" => self.quality_hotspots(&args, &timer),
            "test_gaps" => self.quality_test_gaps(&args, &timer),
            unknown => self.error_response(
                &timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!(
                        "Unknown quality analysis '{unknown}'. Valid: 'repo_health', 'dead_code', 'quality_signal', 'hotspots', 'test_gaps'."
                    ),
                    detail: json!({ "analysis": unknown }),
                },
            ),
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let mut response = inner?;
        if let Some(budget) = args.max_wait_ms
            && elapsed_ms > budget
        {
            if args.partial_ok.unwrap_or(false) {
                if let Some(body) = response.as_object_mut()
                    && let Some(Value::Array(warns)) = body
                        .entry("warnings".to_string())
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()
                        .map(|a| Value::Array(std::mem::take(a)))
                        .and_then(|v| {
                            body.insert("warnings".to_string(), v);
                            body.get_mut("warnings")
                        })
                {
                    warns.push(json!({
                        "code": "BUDGET_EXCEEDED",
                        "message": format!(
                            "analysis took {elapsed_ms} ms which exceeds max_wait_ms={budget}; \
                             returning partial result per partial_ok=true.",
                        ),
                        "detail": { "elapsed_ms": elapsed_ms, "budget_ms": budget },
                    }));
                }
            } else {
                return self.error_response(
                    &timer,
                    ToolError {
                        category: ToolErrorCategory::Internal,
                        code: "DEADLINE_EXCEEDED",
                        message: format!(
                            "analysis took {elapsed_ms} ms which exceeds max_wait_ms={budget}; \
                             pass partial_ok=true to accept partial results.",
                        ),
                        detail: json!({ "elapsed_ms": elapsed_ms, "budget_ms": budget }),
                    },
                );
            }
        }
        if let Some(meta) = response.get_mut("_meta").and_then(Value::as_object_mut) {
            meta.insert("thresholds".to_string(), current_thresholds().as_json());
        }
        Ok(response)
    }

    fn quality_dead_code(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        let granularity = args.granularity.as_deref().unwrap_or("file");
        let min_confidence = args.min_confidence.unwrap_or(0.5);
        // Default false: test functions inside `mod tests` (Rust),
        // `def test_…` (Python), `TestXxx` (Go), etc. are entry-only —
        // invoked by the test harness, not by other indexed code — so
        // flagging them as "no_structural_inbound" is meaningless. Pass
        // `include_tests=true` to inspect test code on its own merits.
        let include_tests = args.include_tests.unwrap_or(false);
        let entry_patterns = args.entry_point_patterns.clone().unwrap_or_default();
        let limit = args.top_n.unwrap_or(25);
        let include_vendored = args.include_vendored.unwrap_or(false)
            || args
                .scope
                .as_deref()
                .map(codesight_language_support::scope_opts_in)
                .unwrap_or(false);
        let paranoid = args.paranoid.unwrap_or(false);
        // ROADMAP P1.1: quality analysis — null baseline.
        // dead_code reachability is fundamentally heuristic: string-dispatch
        // entry points and trait-object call sites can't be ruled out from
        // a static index alone. The `confidence_reason` companion tells
        // callers exactly which heuristic limited the verdict.
        let timer = MetaBuilder::new(0)
            .with_confidence(Confidence::Heuristic)
            .with_confidence_reason(
                "string-literal-dispatch heuristic; manual review for plugin / handler-table dispatch",
            );

        let cache_key = DeadCodeCacheKey {
            source_id: args.source_id.clone(),
            granularity: granularity.to_string(),
            min_confidence_bits: min_confidence.to_bits(),
            include_tests,
            include_vendored,
            paranoid,
            top_n: limit,
            entry_patterns: entry_patterns.clone(),
            max_tokens: args.max_tokens,
        };
        let generation = self.catalog_generation();
        let data = if let Some(cached) = dead_code_cache().lookup(&cache_key, generation) {
            cached
        } else {
            let mut fresh = dead_code::dead_code_payload(
                &store,
                granularity,
                min_confidence,
                include_tests,
                &entry_patterns,
                Some(limit),
                include_vendored,
                paranoid,
            )?;
            if let Some(budget) = args.max_tokens {
                dead_code::pack_dead_code_to_budget(&mut fresh, budget);
            }
            dead_code_cache().store(cache_key, generation, fresh.clone());
            fresh
        };
        // Tier 6.1: actionable_next hints point at the public `references`
        // tool (not the wrapper-internal `inspect_graph` name) so callers
        // can copy-paste the suggestion directly.
        let hints: Vec<Value> = data
            .pointer("/dead_files")
            .or_else(|| data.pointer("/items"))
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .take(3)
                    .filter_map(|row| {
                        let path = row
                            .get("file")
                            .or_else(|| row.get("path"))
                            .and_then(Value::as_str)?;
                        Some(json!({
                            "tool": "references",
                            "args": {
                                "kind": "importers",
                                "path": path,
                            },
                            "reason": format!("Confirm `{path}` is truly unreferenced before deleting."),
                        }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Tier 4.2: top-level `MANUAL_REVIEW_REQUIRED` warning when any
        // flagged file lands on cli/bin/main/hooks paths. The flag is
        // already on the per-row data, but a top-level warning is what
        // surfaces in the calibrated envelope so callers don't act on
        // these without re-checking.
        let manual_review_count = data
            .pointer("/manual_review_required_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut warnings = Vec::new();
        if manual_review_count > 0 {
            warnings.push(crate::service::agent_v1::warnings::warning(
                crate::service::agent_v1::warnings::MANUAL_REVIEW_REQUIRED,
                format!(
                    "{manual_review_count} file(s) flagged dead under cli/, bin/, main.rs, hooks*.rs, or lib.rs. \
                     Externally-invoked entry points may not be statically reachable from `fn main`; \
                     verify with `references kind=importers path=<file>` before acting."
                ),
            ));
        }
        self.success_response_with_meta_extra(
            &timer,
            data,
            warnings,
            [
                ("methodology".to_string(), json!("multi_signal")),
                ("confidence_level".to_string(), json!("medium")),
                ("actionable_next".to_string(), json!(hints)),
            ],
        )
    }

    fn quality_signal_handler(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), timer);
        // ROADMAP P1.1: quality analysis — null baseline.
        // Multi-signal aggregation (structural + agent readiness + heuristic
        // sub-metrics) — partial.
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Partial);

        // The probe is recorded on the cold path; warm hits inside the same
        // generation re-recording it would only echo identical state.
        let cache_key = QualitySignalCacheKey {
            source_id: args.source_id.clone(),
            scope: args.scope.clone(),
        };
        let generation = self.catalog_generation();
        if let Some(cached) = quality_signal_cache().lookup(&cache_key, generation) {
            return self.success_response(&timer, cached);
        }

        let adj = store.dependency_graph_strict()?;
        let cycles = find_cycles(&adj);
        let all_symbols = store.all_symbols_for_analysis()?;
        let readiness_inputs =
            AgentReadinessInputs::build(&all_symbols, &adj, store.root_path().ok().as_deref());
        let gov = quality_signal::compute_governance_signal_precomputed(
            &store,
            &cycles,
            &readiness_inputs,
        )?;
        let suggestion =
            quality_signal::suggestion_for_bottleneck(gov.bottleneck, &gov.structural.coverage);
        let mut data = quality_signal::governance_signal_json(&gov, &suggestion);
        // quality_signal response adds analysis label and coverage block.
        let obj = data.as_object_mut().unwrap();
        obj.insert("analysis".into(), json!("quality_signal"));
        // TL;DR: surface the highest-severity recommendation as a one-line
        // `next_action` so callers don't have to parse the recommendations
        // array to know what to do.
        if let Some(top) = data
            .get("recommendations")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|r| r.get("suggestion"))
            .and_then(|s| s.as_str())
            .map(str::to_string)
        {
            data.as_object_mut()
                .unwrap()
                .insert("next_action".into(), json!(top));
        }
        let obj = data.as_object_mut().unwrap();
        obj.insert(
            "coverage".into(),
            json!({
                "graph_files": gov.structural.coverage.graph_files,
                "graph_edges": gov.structural.coverage.graph_edges,
                "functions_with_source": gov.structural.coverage.functions_with_source,
                "dimensions_available": gov.structural.coverage.dimensions_available,
            }),
        );
        if let Some(scope) = args.scope.as_deref()
            && let Some(scoped) = compute_agent_readiness_for_scope(&readiness_inputs, scope)
        {
            // ROADMAP P2.5: the scoped readiness block used to share the
            // `agent_readiness` key with the global payload block, with
            // values scaled differently. Renaming here removes that
            // collision — global readiness stays at the top level, the
            // scoped sub-signal lives on its own key.
            obj.insert("scope".into(), json!(scope));
            // FIXUP T2.2: scoped.signal.score is f64 in [0.0, 1.0], same
            // range as its dimension siblings. Divided by 10_000.0 the
            // result became ~0.00008; remove the bogus conversion.
            obj.insert(
                "scope_agent_readiness".into(),
                json!({
                    "score": scoped.signal.score,
                    "navigability": scoped.signal.dimensions.navigability,
                    "comprehensibility": scoped.signal.dimensions.comprehensibility,
                    "verifiability": scoped.signal.dimensions.verifiability,
                    "change_safety": scoped.signal.dimensions.change_safety,
                }),
            );
            obj.insert("weakest_dimension".into(), json!(scoped.weakest_dimension));
            obj.insert(
                "top_offenders".into(),
                json!(
                    scoped
                        .top_offenders
                        .into_iter()
                        .map(|offender| json!({
                            "symbol": offender.symbol,
                            "line": offender.line,
                            "cyclomatic": offender.cyclomatic,
                            "nesting_depth": offender.nesting_depth,
                        }))
                        .collect::<Vec<_>>()
                ),
            );
            obj.insert("thresholds_failed".into(), json!(scoped.thresholds_failed));
            self.record_agent_readiness_probe(
                scope,
                scoped.signal.score,
                scoped.weakest_dimension,
                current_thresholds().agent_readiness_min,
            );
        }
        quality_signal_cache().store(cache_key, generation, data.clone());
        self.success_response(&timer, data)
    }
}
