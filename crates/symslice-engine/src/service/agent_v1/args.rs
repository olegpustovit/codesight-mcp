//! Agent-v1 tool argument types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IngestSourcesArgs {
    /// `local_path`, `single_file`, or `git_repo` (unsupported).
    #[serde(default = "default_input_kind")]
    pub input_kind: String,
    /// `code`, `docs`, or `auto` (both indexes for the same `source_id`).
    #[serde(default = "default_domains")]
    pub domains: String,
    pub path: String,
    #[serde(default = "default_incremental")]
    pub incremental: bool,
    #[serde(default)]
    pub enrichment: Vec<String>,
}

fn default_input_kind() -> String {
    "local_path".to_string()
}
fn default_domains() -> String {
    "auto".to_string()
}
fn default_incremental() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct CatalogSourcesArgs {
    #[allow(dead_code)]
    #[serde(default)]
    pub cursor: Option<String>,
    /// ROADMAP P2.4: drop `capabilities`, `languages`, and `root_hint` from
    /// each entry when `true`. Cuts the warmup payload ~50% for sessions
    /// that only need `{source_id, status, indexed_at, file_count}`.
    #[serde(default)]
    pub lightweight: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchCatalogArgs {
    pub source_id: String,
    pub query: String,
    #[serde(default)]
    pub domain: Option<String>,
    /// `symbol`, `section`, `file`.
    #[serde(default)]
    pub unit_kinds: Option<Vec<String>>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
    #[serde(default)]
    pub file_pattern: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default = "crate::service::args::default_max_results")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub query_mode: Option<String>,
    /// Surrounding lines for file hits, and symbol/section bodies when requested (clamped).
    #[serde(default)]
    pub context_lines: Option<usize>,
    /// Include full symbol/section body text in each hit.
    #[serde(default)]
    pub include_body: Option<bool>,
    /// Cap total serialized size of returned `items` (~bytes / BYTES_PER_TOKEN). Alias: `token_budget`.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// Augment hits with a nucleo-matcher fuzzy lane over symbol names.
    #[serde(default)]
    pub fuzzy: Option<bool>,
    #[serde(default)]
    pub repo_path: Option<String>,
    /// Enable dense-embedding retrieval as an RRF-fused augmentation to BM25.
    /// When semantic fails (model unavailable, vectors missing) the response
    /// still succeeds and includes a `SEMANTIC_FALLBACK` warning.
    #[serde(default)]
    pub semantic: Option<bool>,
    /// Sort-by mode: "relevance" (default), "centrality", etc.
    #[serde(default)]
    pub sort_by: Option<String>,
    /// Output mode: "items" (default), "count", or "files_with_matches".
    #[serde(default)]
    pub output_mode: Option<String>,
    /// Accept `pattern` as alias for `query` in regex mode.
    #[serde(default)]
    pub pattern: Option<String>,
    /// For `natural_language` mode: include test files / fixtures / benches in
    /// the results. Defaults to `false` for NL queries so "how is X done"
    /// surfaces production code over the test asserting it. `keyword` / `regex`
    /// modes always include tests (the caller asked for the literal match).
    #[serde(default)]
    pub include_tests: Option<bool>,
    /// FIXUP T1.3: when true in `keyword` mode, match only against symbol
    /// `name` / `qualified_name`, ignoring body, signature, summary, and
    /// docstring features. Identifier searches become focused instead of
    /// drowning in body matches.
    #[serde(default)]
    pub name_only: Option<bool>,
    /// When true, include matches under vendored / asset / fixture paths
    /// (the union of every registered language's
    /// [`symslice_language_support::Language::vendor_segments`] plus the
    /// universal segments) and oversized JSON dumps. Default `false`:
    /// those paths are deprioritized to the tail of the ranking so real
    /// source outranks tokenizer vocab and fixture files. An explicit
    /// `scope=` that starts with an excluded prefix also opts in.
    #[serde(default)]
    pub include_vendored: Option<bool>,
    /// Tier 2.1: optional ripgrep-style per-file cap for `query_mode=regex`.
    /// `limit` is always a global item cap; if you also want "at most N
    /// hits per file" semantics (so one verbose file doesn't eat the whole
    /// budget), set this. Ignored for non-regex modes.
    #[serde(default)]
    pub per_file_limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ContentRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub stable_id: Option<String>,
    pub kind: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub end_line: Option<u32>,
    #[serde(default)]
    pub new_name: Option<String>,
    /// Bare symbol name — used in combination with `path` (preferred) or on
    /// its own for unique names across the index. Lets callers that know the
    /// call site but not its `start_line` / `stable_id` skip a lookup step.
    #[serde(default, alias = "name")]
    pub symbol_name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadContentArgs {
    pub source_id: String,
    pub refs: Vec<ContentRef>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub verify: bool,
    #[serde(default)]
    pub context_lines: i64,
    #[serde(default)]
    pub repo_path: Option<String>,
    /// Soft token budget for the combined item payload. Once exceeded, later
    /// refs are stubbed with a structured `truncated: true` marker instead of
    /// returning unreadable JSON-escaped previews (FEEDBACK.md §3a). Alias:
    /// `token_budget`.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// If a reindex is mid-flight for this source_id, block until it
    /// completes or `max_wait_ms` elapses (FEEDBACK.md §5b). Default: false
    /// — the handler annotates STALE_DATA_WARNING and returns immediately.
    #[serde(default)]
    pub wait_for_fresh: Option<bool>,
    #[serde(default)]
    pub max_wait_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct InspectStructureArgs {
    pub source_id: String,
    pub view: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    /// Optional token budget (~4 bytes/token) for repo_outline.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub repo_path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct InspectGraphArgs {
    pub source_id: String,
    pub graph: String,
    #[serde(default)]
    pub repo_path: Option<String>,
    /// For `importers` mode: file path to find importers for.
    #[serde(default)]
    pub file_path: Option<String>,
    /// For `find_references` mode: identifier to search for.
    #[serde(default)]
    pub identifier: Option<String>,
    /// For `find_references` batch mode: multiple identifiers.
    #[serde(default)]
    pub identifiers: Option<Vec<String>>,
    /// For `find_references` mode: include calling symbol chain info.
    #[serde(default)]
    pub include_call_chain: Option<bool>,
    /// Enable cross-repo analysis.
    #[serde(default)]
    pub cross_repo: Option<bool>,
    /// Path-prefix / substring inclusion filter.
    #[serde(default)]
    pub scope: Option<String>,
    /// Path prefixes / substrings to skip (complements `scope`).
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
    /// Glob-ish pattern to filter edge endpoints by file path. Supports
    /// `*` / `**` / `?` like a conservative gitignore.
    #[serde(default)]
    pub file_pattern: Option<String>,
    /// `find_references`: when true, include hits in data-format files
    /// (`json`, `yaml`, `toml`, `xml`, `md`). Default false — those files
    /// are almost never the right answer for "who references fn X".
    #[serde(default)]
    pub include_non_code: Option<bool>,
    /// When true, include references found under vendored / asset /
    /// fixture paths and oversized JSON dumps. Default false.
    #[serde(default)]
    pub include_vendored: Option<bool>,
    /// Tier 2.1: global cap on returned items (semantic + candidate combined).
    /// Surfaces a `TRUNCATED` warning when the unbounded count exceeds this.
    /// Defaults to 50 when unset (matches `references` schema default).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateQualityArgs {
    pub source_id: String,
    pub analysis: String,
    #[serde(default)]
    pub repo_path: Option<String>,
    // dead_code fields
    #[serde(default)]
    pub granularity: Option<String>,
    #[serde(default)]
    pub min_confidence: Option<f64>,
    #[serde(default)]
    pub include_tests: Option<bool>,
    #[serde(default)]
    pub entry_point_patterns: Option<Vec<String>>,
    // hotspots fields
    #[serde(default)]
    pub min_complexity: Option<u64>,
    #[serde(default)]
    pub top_n: Option<usize>,
    /// Optional token budget for the serialized response body. Alias
    /// `token_budget`.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// If true, `hotspots` scans conventional dirs (scripts/, tests/,
    /// examples/, benches/, build.rs, fixtures/, …). Defaults to false so
    /// codegen helpers and fixture noise stop dominating the top of the
    /// ranking (FEEDBACK.md Phase 4a).
    #[serde(default)]
    pub include_conventional: Option<bool>,
    #[serde(default)]
    pub scope: Option<String>,
    /// Cap wall-clock time for the analysis. No value = unbounded.
    #[serde(default)]
    pub max_wait_ms: Option<u64>,
    /// Accept a truncated-but-well-formed envelope on deadline expiry.
    /// When false (default) the handler prefers to error with
    /// `DEADLINE_EXCEEDED`.
    #[serde(default)]
    pub partial_ok: Option<bool>,
    /// When true, include vendored / asset / fixture files and oversized
    /// JSON dumps in the analysis. Default false.
    #[serde(default)]
    pub include_vendored: Option<bool>,
    /// `dead_code` only: when true, drop the confidence floor to 0,
    /// surface weak signals (`weak_string_dispatch_match`,
    /// `test_only_alive`), and always run the symbol-level pass.
    #[serde(default)]
    pub paranoid: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ManageIndexesArgs {
    pub action: String,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub root_hint: Option<String>,
    #[serde(default)]
    pub domains: Option<String>,
    #[serde(default)]
    pub snapshot_name: Option<String>,
}
