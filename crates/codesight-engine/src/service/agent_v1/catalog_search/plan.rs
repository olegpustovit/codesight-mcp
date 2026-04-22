//! Argument coercion, validation, and the resolved `SearchPlan`.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

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
        let want_symbol = kinds
            .map(|k| k.iter().any(|x| x == "symbol"))
            .unwrap_or(true)
            && domain != "docs";
        let want_section = kinds
            .map(|k| k.iter().any(|x| x == "section"))
            .unwrap_or(false)
            || domain == "docs"
            || domain == "hybrid";
        let want_file = kinds
            .map(|k| k.iter().any(|x| x == "file"))
            .unwrap_or(false);
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
        })
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct LexicalStats {
    pub(super) more_file_hits_exist: bool,
    pub(super) fuzzy_applied: bool,
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

pub(super) fn build_meta_extras(
    plan: &SearchPlan,
    fuzzy_applied: bool,
    semantic_requested: bool,
) -> Vec<(String, Value)> {
    vec![
        ("domain_effective".to_string(), json!(plan.domain)),
        (
            "query_mode_effective".to_string(),
            json!(plan.query_mode_effective),
        ),
        ("domain_explicit".to_string(), json!(plan.domain_explicit)),
        (
            "estimate_method".to_string(),
            json!("byte_approx_candidate_files"),
        ),
        (
            "baseline_scope".to_string(),
            json!("candidate_files_materialized_pre_output"),
        ),
        (
            "unit_kinds_effective".to_string(),
            json!({
                "symbol": plan.flags.want_symbol,
                "section": plan.flags.want_section,
                "file": plan.flags.want_file,
            }),
        ),
        ("fuzzy_applied".to_string(), json!(fuzzy_applied)),
        ("semantic_applied".to_string(), json!(semantic_requested)),
    ]
}

pub(super) fn estimate_candidate_file_bytes(
    code_store: Option<&IndexStore>,
    doc_store: Option<&DocStore>,
    items: &[Value],
) -> Result<u64, ToolError> {
    let mut code_paths = BTreeSet::new();
    let mut doc_paths = BTreeSet::new();

    for item in items {
        let Some(path) = item.pointer("/ref/path").and_then(|value| value.as_str()) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        match item.pointer("/ref/domain").and_then(|value| value.as_str()) {
            Some("docs") => {
                doc_paths.insert(path.to_string());
            }
            _ => {
                code_paths.insert(path.to_string());
            }
        }
    }

    let mut total = 0u64;
    if let Some(store) = code_store
        && !code_paths.is_empty()
    {
        let paths = code_paths.into_iter().collect::<Vec<_>>();
        total = total.saturating_add(
            store
                .sum_file_sizes_for_paths(&paths)
                .map_err(index_error)?,
        );
    }
    if let Some(store) = doc_store
        && !doc_paths.is_empty()
    {
        let paths = doc_paths.into_iter().collect::<Vec<_>>();
        total = total.saturating_add(
            store
                .sum_file_sizes_for_paths(&paths)
                .map_err(index_error)?,
        );
    }

    Ok(total)
}
