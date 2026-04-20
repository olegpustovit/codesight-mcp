//! Agent-v1 tool argument types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const DEFAULT_FILE_TREE_MAX_FILES: usize = 500;

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
    /// Augment FTS hits with case-insensitive name substring matches (deduped).
    #[serde(default)]
    pub fuzzy: Option<bool>,
    /// Trigram Jaccard threshold for fuzzy augmentation (default 0.3). When set while BM25
    /// returns no symbol hits, fuzzy augmentation runs even if `fuzzy` is false.
    #[serde(default)]
    pub fuzzy_threshold: Option<f64>,
    /// When set, symbols within this Levenshtein distance (on a normalized compare key) qualify.
    #[serde(default)]
    pub max_edit_distance: Option<u32>,
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
    #[serde(default)]
    pub class_path: Option<String>,
    /// Cap the number of **files** listed (`file_tree` only; default 500).
    #[serde(default)]
    pub max_files: Option<usize>,
    /// Optional token budget (~4 bytes/token) for file_tree / repo_outline.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub repo_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InspectGraphArgs {
    pub source_id: String,
    pub graph: String,
    #[serde(default)]
    pub r#ref: Option<Value>,
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub repo_path: Option<String>,
    /// For `module_coupling` graph mode: file to analyze.
    #[serde(default)]
    pub module_path: Option<String>,
    /// For `layer_violations` graph mode: layer rules.
    #[serde(default)]
    pub rules: Option<Vec<codesight_analysis::graph::LayerDef>>,
    /// For `calls` mode: symbol to analyze.
    #[serde(default)]
    pub symbol_id: Option<String>,
    /// For `calls` mode: "callers", "callees", or "both" (default).
    #[serde(default)]
    pub direction: Option<String>,
    /// For `calls`/`centrality` mode: traversal depth.
    #[serde(default)]
    pub depth: Option<usize>,
    /// For `centrality` mode: "pagerank" or "degree".
    #[serde(default)]
    pub algorithm: Option<String>,
    /// For `centrality` mode: top N results.
    #[serde(default)]
    pub top_n: Option<usize>,
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
    /// Enable cross-repo analysis for importers/dependencies.
    #[serde(default)]
    pub cross_repo: Option<bool>,
    /// For `resolve_fqn` mode: fully-qualified name.
    #[serde(default)]
    pub fqn: Option<String>,
    /// Optional token budget for the serialized `edges`/`nodes` in
    /// `dependencies` (≈4 bytes/token). Alias `token_budget`.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// Path-prefix / substring inclusion filter. Applies to
    /// `find_references`, `imports`, `importers`, `references`, `related`,
    /// and `calls` (FEEDBACK.md §6b).
    #[serde(default)]
    pub scope: Option<String>,
    /// Path prefixes / substrings to skip (complements `scope`). Applies to
    /// the same graph modes as `scope`.
    #[serde(default)]
    pub exclude: Option<Vec<String>>,
    /// Glob-ish pattern to filter edge endpoints by file path. Supports
    /// `*` / `**` / `?` like a conservative gitignore. Applies to the same
    /// graph modes as `scope` (FEEDBACK.md §6b).
    #[serde(default)]
    pub file_pattern: Option<String>,
    /// `find_references`: when true, include hits in data-format files
    /// (`json`, `yaml`, `toml`, `xml`, `md`). Default false — those files
    /// are almost never the right answer for "who references fn X".
    #[serde(default)]
    pub include_non_code: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateImpactArgs {
    pub source_id: String,
    pub analysis: String,
    #[serde(default)]
    pub refs: Option<Vec<ContentRef>>,
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub depth: Option<u64>,
    #[serde(default)]
    pub include_depth_scores: Option<bool>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub days: Option<u64>,
    /// Symbol name for impact_preview / blast_radius (alternative to refs).
    #[serde(default)]
    pub symbol_id: Option<String>,
    /// Call hierarchy depth for blast_radius (callers lookup).
    #[serde(default)]
    pub call_depth: Option<usize>,
    /// Enable cross-repo analysis.
    #[serde(default)]
    pub cross_repo: Option<bool>,
    /// `path_rename`: existing fully-qualified Rust path to rewrite.
    #[serde(default)]
    pub from: Option<String>,
    /// `path_rename`: replacement fully-qualified Rust path.
    #[serde(default)]
    pub to: Option<String>,
    /// `path_rename`: when true, write edits to disk and reindex.
    #[serde(default)]
    pub apply: Option<bool>,
    /// `path_rename`: behavior for partial grouped imports.
    #[serde(default)]
    pub handle_partial_group_imports: Option<String>,
    /// `path_rename`: optional path/file prefix scope.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EvaluateQualityArgs {
    pub source_id: String,
    pub analysis: String,
    #[serde(default)]
    pub refs: Option<Vec<ContentRef>>,
    #[serde(default)]
    pub repo_path: Option<String>,
    // dead_code / dead_code_v2 fields
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
    // refactor_targets fields
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub min_callers: Option<u64>,
    /// For repo-wide `refactor_targets`: how many hottest files to scan
    /// when `file_path` is omitted.
    #[serde(default)]
    pub top_files: Option<usize>,
    /// Optional token budget for the serialized response body. Alias
    /// `token_budget`.
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// Max `caller_files` entries per `refactor_targets` candidate
    /// (FEEDBACK.md Phase 4b). When truncated, `caller_files_total` carries
    /// the untruncated count. Defaults to 10.
    #[serde(default)]
    pub top_caller_files: Option<usize>,
    /// If true, `hotspots` / `refactor_targets` scan conventional dirs
    /// (scripts/, tests/, examples/, benches/, build.rs, fixtures/, …).
    /// Defaults to false so codegen helpers and fixture noise stop
    /// dominating the top of the ranking (FEEDBACK.md Phase 4a).
    #[serde(default)]
    pub include_conventional: Option<bool>,
    // regression_check fields
    #[serde(default)]
    pub snapshot_name: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssembleContextArgs {
    pub source_id: String,
    pub mode: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub refs: Option<Vec<ContentRef>>,
    #[serde(default = "default_ctx_limit")]
    pub limit: usize,
    #[serde(default = "default_include_children")]
    pub include_children: bool,
    #[serde(default, alias = "token_budget")]
    pub max_tokens: Option<u32>,
    /// `context_bundle`: `compact`, `most_relevant`, or unset for full bodies.
    #[serde(default)]
    pub budget_strategy: Option<String>,
    #[serde(default)]
    pub include_budget_report: Option<bool>,
    /// `ranked`: accepted strategy label (default `bm25`); stored for reporting only today.
    #[serde(default)]
    pub strategy: Option<String>,
    /// `ranked`: restrict hits to these symbol kinds (`class`, `function`, …).
    #[serde(default, alias = "include_kinds")]
    pub kind_filter: Option<Vec<String>>,
    /// `rename_plan`: existing name to rename. Defaults to `query`.
    #[serde(default)]
    pub old_name: Option<String>,
    /// `rename_plan`: proposed new name.
    #[serde(default)]
    pub new_name: Option<String>,
    #[serde(default)]
    pub repo_path: Option<String>,
}

/// `get_ranked_context` reference oracle caps query length at 500 chars.
pub(crate) const RANKED_CONTEXT_MAX_QUERY_LEN: usize = 500;

fn default_include_children() -> bool {
    true
}

fn default_ctx_limit() -> usize {
    5
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
