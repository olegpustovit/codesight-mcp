//! Argument coercion, validation, and the resolved `SearchPlan`.

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};

use super::intent::{QueryIntent, caller_opted_into_doc_sections, classify_query};

pub(super) struct KindFlags {
    pub(super) want_symbol: bool,
    pub(super) want_section: bool,
    pub(super) want_file: bool,
}

impl KindFlags {
    pub(super) fn from_args(args: &SearchCatalogArgs, domain: &str) -> Self {
        let regex_mode = args.query_mode.as_deref() == Some("regex");
        if regex_mode {
            return Self {
                want_symbol: false,
                want_section: false,
                want_file: true,
            };
        }
        let kinds = args.unit_kinds.as_ref();
        // ROADMAP P3.3: `doc_section` is a first-class alias for `section`.
        // Agents searching docs can pass `unit_kinds=["doc_section"]`
        // without needing to remember `domain=hybrid` implies sections.
        let matches = |want: &str| -> bool {
            kinds
                .map(|k| {
                    k.iter()
                        .any(|x| x == want || (want == "section" && x == "doc_section"))
                })
                .unwrap_or(false)
        };
        let want_symbol = kinds
            .map(|k| k.iter().any(|x| x == "symbol"))
            .unwrap_or(true)
            && domain != "docs";
        let want_section = matches("section") || domain == "docs" || domain == "hybrid";
        let want_file = matches("file");
        Self {
            want_symbol,
            want_section,
            want_file,
        }
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct CodeSideStats {
    pub(super) more_file_hits: bool,
    pub(super) fuzzy_applied: bool,
    pub(super) name_only_applied: bool,
    /// Tier 1.3: bytes of file content scanned during this code-side pass
    /// (only meaningful for `query_mode=regex`, which routes to file text).
    pub(super) regex_bytes_scanned: u64,
    /// Tier 1.3: file count actually scanned (post scope/exclude filters).
    pub(super) regex_files_scanned: usize,
}

pub(super) struct SearchPlan {
    pub(super) output_mode: String,
    pub(super) scope: Option<String>,
    pub(super) offset: usize,
    pub(super) lim: usize,
    pub(super) query_mode_effective: String,
    pub(super) domain_explicit: bool,
    pub(super) domain: String,
    pub(super) flags: KindFlags,
    pub(super) per_kind_cap: usize,
    /// Coarse intent: `Code` penalizes doc-section hits, `Doc` boosts them,
    /// `Unknown` leaves them alone. See `intent::classify_query`.
    pub(super) query_intent: QueryIntent,
    /// True when the caller passed `unit_kinds=["doc_section"]` /
    /// `["section"]` and therefore opted in to doc-section retrieval — the
    /// intent penalty is bypassed in that case.
    pub(super) doc_section_opt_in: bool,
}

impl SearchPlan {
    pub(super) fn build(args: &mut SearchCatalogArgs) -> Result<Self, ToolError> {
        let output_mode = coerce_search_args(args)?;
        validate_presearch(args)?;
        let scope = args.scope.clone().or(args.file_pattern.clone());
        let counting = output_mode == "count" || output_mode == "files_with_matches";
        let offset = args.offset;
        let lim = if counting {
            args.limit.clamp(1, 10_000)
        } else {
            args.limit.clamp(1, 50)
        };
        let query_mode_effective = args
            .query_mode
            .clone()
            .unwrap_or_else(|| "keyword".to_string())
            .to_ascii_lowercase();
        let domain_explicit = args.domain.is_some();
        let domain = args
            .domain
            .clone()
            .map(|d| d.to_ascii_lowercase())
            .unwrap_or_else(|| match query_mode_effective.as_str() {
                "natural_language" => "hybrid".to_string(),
                _ => "code".to_string(),
            });
        let flags = KindFlags::from_args(args, &domain);
        let request_cap = lim.saturating_add(offset);
        let per_kind_cap = if domain == "hybrid" {
            (request_cap / 2).max(1)
        } else {
            request_cap
        };
        let query_intent = classify_query(&args.query);
        let doc_section_opt_in = caller_opted_into_doc_sections(args.unit_kinds.as_ref());
        Ok(Self {
            output_mode,
            scope,
            offset,
            lim,
            query_mode_effective,
            domain_explicit,
            domain,
            flags,
            per_kind_cap,
            query_intent,
            doc_section_opt_in,
        })
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct LexicalStats {
    pub(super) more_file_hits_exist: bool,
    pub(super) fuzzy_applied: bool,
    pub(super) name_only_applied: bool,
    /// Total bytes of file content scanned during a regex pass. 0 for
    /// keyword/NL (not meaningful — those hit indexed FTS).
    pub(super) regex_bytes_scanned: u64,
    /// Number of files that ended up actually being read during a regex pass.
    pub(super) regex_files_scanned: usize,
}

#[derive(Default, Clone, Copy)]
pub(super) struct SearchCounts {
    pub(super) total_matches: usize,
    pub(super) total_files_with_matches: usize,
}

impl SearchCounts {
    pub(super) fn add(&mut self, m: usize, f: usize) {
        self.total_matches += m;
        self.total_files_with_matches += f;
    }
}

fn validate_presearch(args: &SearchCatalogArgs) -> Result<(), ToolError> {
    if let Some(ref sort_by) = args.sort_by {
        let valid = [
            "relevance",
            "centrality",
            "name",
            "file",
            "line",
            "combined",
        ];
        if !valid.contains(&sort_by.as_str()) {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: format!(
                    "Invalid sort_by value '{}'. Valid values: {:?}",
                    sort_by, valid
                ),
                detail: json!({ "sort_by": sort_by }),
            });
        }
    }
    Ok(())
}

/// Coerce `pattern` alias into `query` and validate `output_mode`.
fn coerce_search_args(args: &mut SearchCatalogArgs) -> Result<String, ToolError> {
    if let Some(pattern) = args.pattern.take() {
        if !args.query.is_empty() {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "INVALID_REQUEST",
                message: "Cannot set both `query` and `pattern`. Use one or the other.".into(),
                detail: json!({}),
            });
        }
        args.query = pattern;
    }
    let output_mode = args
        .output_mode
        .as_deref()
        .unwrap_or("items")
        .to_ascii_lowercase();
    if !matches!(
        output_mode.as_str(),
        "items" | "count" | "files_with_matches"
    ) {
        return Err(ToolError {
            category: ToolErrorCategory::InvalidArgument,
            code: "INVALID_REQUEST",
            message: format!(
                "Invalid output_mode '{}'. Valid: items, count, files_with_matches.",
                output_mode
            ),
            detail: json!({ "output_mode": output_mode }),
        });
    }
    Ok(output_mode)
}

/// Inputs to [`build_meta_extras`]. Keeps the signature flat without
/// blowing past clippy's `too_many_arguments` ceiling.
pub(super) struct MetaExtras<'a> {
    pub plan: &'a SearchPlan,
    pub fuzzy_applied: bool,
    pub semantic_requested: bool,
    pub name_only_applied: bool,
    pub doc_section_penalty: f64,
    pub doc_section_rank_demotion: usize,
    pub regex_bytes_scanned: u64,
    pub regex_files_scanned: usize,
}

pub(super) fn build_meta_extras(input: MetaExtras<'_>) -> Vec<(String, Value)> {
    let MetaExtras {
        plan,
        fuzzy_applied,
        semantic_requested,
        name_only_applied,
        doc_section_penalty,
        doc_section_rank_demotion,
        regex_bytes_scanned,
        regex_files_scanned,
    } = input;
    let mut extras: Vec<(String, Value)> = vec![
        ("domain_effective".to_string(), json!(plan.domain)),
        (
            "query_mode_effective".to_string(),
            json!(plan.query_mode_effective),
        ),
        ("domain_explicit".to_string(), json!(plan.domain_explicit)),
        // ROADMAP P1.1: baseline is now `rg -n`-equivalent output
        // (path:line:snippet per hit), capped at the caller's `limit`. This
        // is what an agent would have paid without dowser, not a
        // fabricated "read every matched file" number.
        (
            "estimate_method".to_string(),
            json!("rg_equivalent_output_bytes"),
        ),
        (
            "baseline_scope".to_string(),
            json!("path_line_snippet_per_returned_hit"),
        ),
    ];
    // Truthful meta: regex mode ignores `unit_kinds` (the file-text scan
    // does not honor symbol/section discrimination), so emitting
    // `unit_kinds_effective` would lie about what the engine actually
    // filtered on. Omit it for regex; emit for keyword/natural_language.
    if plan.query_mode_effective != "regex" {
        extras.push((
            "unit_kinds_effective".to_string(),
            json!({
                "symbol": plan.flags.want_symbol,
                "section": plan.flags.want_section,
                "file": plan.flags.want_file,
            }),
        ));
    } else {
        // Tier 1.3: regex-mode meta. `tokens_saved` is meaningless here
        // (regex returns the same `path:line:snippet` rows `rg` would, so
        // the savings baseline equals the response). The honest signal is
        // *what we read*: bytes / files scanned. Both are 0 when the
        // pattern failed to compile (the synthetic `regex_error` early
        // return).
        extras.push(("bytes_scanned".to_string(), json!(regex_bytes_scanned)));
        extras.push(("files_scanned".to_string(), json!(regex_files_scanned)));
    }
    extras.extend([
        ("fuzzy_applied".to_string(), json!(fuzzy_applied)),
        ("semantic_applied".to_string(), json!(semantic_requested)),
        ("name_only_applied".to_string(), json!(name_only_applied)),
        // Surface the intent classifier's decision so callers can see why a
        // doc section ranked where it did. `doc_section_penalty` echoes the
        // *effective* multiplier — already accounts for the
        // `unit_kinds=["doc_section"]` opt-in bypass and the gap-aware
        // softening (0.4× hard, 0.5× when best doc/code raw scores are
        // within 0.3).
        (
            "query_intent".to_string(),
            json!(plan.query_intent.as_str()),
        ),
        (
            "doc_section_penalty".to_string(),
            json!(doc_section_penalty),
        ),
        // Rank-based demotion applied AFTER the score multiplier when intent
        // is `code` and at least 2 items are returned. Doc-sections move
        // down by `max(N/2, 3)` positions even if the multiplier alone
        // didn't pull their score below the best code answer. Zero when
        // intent != code or list is too short to bother.
        (
            "doc_section_rank_demotion".to_string(),
            json!(doc_section_rank_demotion),
        ),
    ]);
    extras
}
