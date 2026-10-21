//! `search_catalog` response shaping: offset, `max_tokens` packing, server
//! byte cap, and mode-specific JSON envelope (items / count / files_with_matches).

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::util::pack_catalog_items_to_token_budget;
use crate::service::savings::MetaBuilder;

use super::plan::SearchCounts;

/// Server-side serialization byte cap for the `items` payload. We stay well
/// below the MCP client's overflow-to-file guard so broad queries trim on
/// the server side with a `truncated_by=["byte_budget"]` hint instead of
/// returning a giant `.txt` pointer.
///
/// Tier 2.1: bumped from 12 KB → 24 KB so regex hits with line-context
/// don't get pinched down to 1 item before the caller's `limit` is honored.
const MAX_RESPONSE_BYTES: usize = 24_000;

/// Tier 2.1: never byte-cap the response below this many items. `limit` is
/// the caller's contract; if it can't fit at the requested context width,
/// we'd rather return the count and let the caller drop `context_lines` or
/// pass `max_tokens` than silently shrink to 1 hit.
const BYTE_CAP_ITEM_FLOOR: usize = 3;

pub(super) struct FormatCtx<'a> {
    pub(super) output_mode: &'a str,
    pub(super) query: &'a str,
    pub(super) query_mode_effective: &'a str,
    pub(super) offset: usize,
    pub(super) lim: usize,
    pub(super) max_tokens: Option<u32>,
    pub(super) more_file_hits_exist: bool,
    pub(super) counts: SearchCounts,
    /// ROADMAP P1.4 — second lane of hits when `fuzzy=true` was explicit.
    /// Empty when the caller did not ask for fuzzy or the fuzzy lane has
    /// nothing to report.
    pub(super) fuzzy_items: Vec<Value>,
}

/// Format collected items for count / files_with_matches / items modes.
pub(super) fn format_output(
    svc: &CodeService,
    ctx: &FormatCtx<'_>,
    mut items: Vec<Value>,
    timer: &MetaBuilder,
    mut warnings: Vec<Value>,
    mut extra: Vec<(String, Value)>,
) -> anyhow::Result<Value> {
    let total_matches = ctx.counts.total_matches;
    let total_files_with_matches = ctx.counts.total_files_with_matches;
    match ctx.output_mode {
        "count" => {
            let data = json!({
                "count": total_matches,
                "files": total_files_with_matches,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            extra.push(("truncated".to_string(), json!(false)));
            extra.push(("truncated_by".to_string(), json!(Vec::<&str>::new())));
            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
        "files_with_matches" => {
            // FIXUP T1.7: previously `file_counts` was built from the
            // already-truncated `items` slice and `truncated` was always
            // `false`. Count files from the full items list (pre-offset),
            // then emit `truncated` when the returned slice is narrower
            // than the full file set the caller would expect given the
            // upstream `total_files_with_matches`.
            let mut file_counts: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for item in &items {
                if let Some(path) = item.pointer("/ref/path").and_then(|v| v.as_str()) {
                    *file_counts.entry(path.to_string()).or_default() += 1;
                }
            }
            let total_distinct_files = file_counts.len();
            let file_items: Vec<Value> = file_counts
                .into_iter()
                .map(|(file, count)| json!({ "file": file, "count": count }))
                .collect();
            let file_items = file_items
                .into_iter()
                .skip(ctx.offset)
                .take(ctx.lim)
                .collect::<Vec<_>>();
            // `total_files_with_matches` comes from storage count_symbols /
            // count_sections and is authoritative for what a plain `rg`
            // would surface; `total_distinct_files` is what our already-
            // capped hit set contains. Take the larger of the two to avoid
            // undercounting when the upstream counter is unavailable.
            let known_total = total_files_with_matches.max(total_distinct_files);
            let returned_len = file_items.len();
            let truncated_by_cap = returned_len < total_distinct_files.saturating_sub(ctx.offset);
            let truncated_by_upstream = returned_len < known_total.saturating_sub(ctx.offset);
            let is_truncated = truncated_by_cap || truncated_by_upstream;
            let mut truncated_by: Vec<&'static str> = Vec::new();
            if is_truncated {
                truncated_by.push("limit");
            }
            let mut data = json!({
                "files": file_items,
                "total_matches": total_matches,
                "total_files_with_matches": known_total,
            });
            if is_truncated && let Value::Object(ref mut map) = data {
                map.insert("truncated".to_string(), json!(true));
                map.insert("truncated_by".to_string(), json!(truncated_by.clone()));
            }
            extra.push(("truncated".to_string(), json!(is_truncated)));
            extra.push(("truncated_by".to_string(), json!(truncated_by)));
            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
        _ => {
            let mut truncated_by: Vec<&'static str> = Vec::new();
            if ctx.offset > 0 {
                items = items.into_iter().skip(ctx.offset).collect();
            }
            if let Some(mt) = ctx.max_tokens {
                let (packed, bt) = pack_catalog_items_to_token_budget(items, mt);
                items = packed;
                if bt {
                    truncated_by.push("token_budget");
                }
            }

            let lim = ctx.lim;
            let pre_cap_exceeds_lim = total_matches > items.len().max(lim);
            let overflow = items.len() > lim;
            if overflow || pre_cap_exceeds_lim || ctx.more_file_hits_exist {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results omitted by global/per-file limit (global limit {lim})"),
                }));
                truncated_by.push("limit");
            }
            items.truncate(lim);

            // Tier 2.1: byte-cap pops items only down to the floor — never
            // below `limit` (or `BYTE_CAP_ITEM_FLOOR`, whichever is smaller).
            // Keeping a floor preserves the caller's `limit` contract; the
            // byte cap is a server-side guard against giant responses, not a
            // way to silently narrow to 1 hit.
            let mut byte_budgeted = false;
            let floor = ctx.lim.min(items.len()).clamp(1, BYTE_CAP_ITEM_FLOOR);
            while items.len() > floor
                && serde_json::to_vec(&items).map(|v| v.len()).unwrap_or(0) > MAX_RESPONSE_BYTES
            {
                items.pop();
                byte_budgeted = true;
            }
            if byte_budgeted {
                truncated_by.push("byte_budget");
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!(
                        "result trimmed to stay under {MAX_RESPONSE_BYTES}-byte server cap; pass `max_tokens=`, drop `context_lines=`, or narrow via `scope=` to keep more items."
                    ),
                }));
            }

            let meta_truncated = !truncated_by.is_empty();
            let mut data = json!({
                "items": items,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            if !ctx.fuzzy_items.is_empty()
                && let Value::Object(ref mut map) = data
            {
                // ROADMAP P1.4: two-lane response — exact BM25 hits live in
                // `items`, fuzzy candidates live in `fuzzy` so the agent can
                // pick without string-matching `match_type`.
                map.insert("fuzzy".to_string(), json!(ctx.fuzzy_items.clone()));
            }
            if meta_truncated && let Value::Object(ref mut map) = data {
                map.insert("truncated".to_string(), json!(true));
                map.insert("truncated_by".to_string(), json!(truncated_by));
            }
            extra.push(("truncated".to_string(), json!(meta_truncated)));
            extra.push(("truncated_by".to_string(), json!(truncated_by)));

            if ctx.query_mode_effective == "keyword" && items.len() <= 3 {
                for item in items.iter().take(3) {
                    if let Some(title) = item.pointer("/ref/title").and_then(|v| v.as_str())
                        && title.eq_ignore_ascii_case(ctx.query)
                    {
                        extra.push((
                            "suggestion".to_string(),
                            json!(format!(
                                "For complete reference tracking, use `inspect_graph graph=find_references identifier=\"{}\"`.",
                                ctx.query
                            )),
                        ));
                        break;
                    }
                }
            }

            svc.success_response_with_meta_extra(timer, data, warnings, extra)
        }
    }
}
