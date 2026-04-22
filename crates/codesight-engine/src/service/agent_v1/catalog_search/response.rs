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
const MAX_RESPONSE_BYTES: usize = 12_000;

pub(super) struct FormatCtx<'a> {
    pub(super) output_mode: &'a str,
    pub(super) query: &'a str,
    pub(super) query_mode_effective: &'a str,
    pub(super) offset: usize,
    pub(super) lim: usize,
    pub(super) max_tokens: Option<u32>,
    pub(super) more_file_hits_exist: bool,
    pub(super) counts: SearchCounts,
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
            let mut file_counts: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for item in &items {
                if let Some(path) = item.pointer("/ref/path").and_then(|v| v.as_str()) {
                    *file_counts.entry(path.to_string()).or_default() += 1;
                }
            }
            let file_items: Vec<Value> = file_counts
                .into_iter()
                .map(|(file, count)| json!({ "file": file, "count": count }))
                .collect();
            let file_items = file_items
                .into_iter()
                .skip(ctx.offset)
                .take(ctx.lim)
                .collect::<Vec<_>>();
            let data = json!({
                "files": file_items,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
            extra.push(("truncated".to_string(), json!(false)));
            extra.push(("truncated_by".to_string(), json!(Vec::<&str>::new())));
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
            if overflow {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results truncated to limit {lim}"),
                }));
                items.truncate(lim);
            } else if pre_cap_exceeds_lim {
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!("results truncated to limit {lim}"),
                }));
            }
            if overflow || pre_cap_exceeds_lim || ctx.more_file_hits_exist {
                truncated_by.push("limit");
            }

            let mut byte_budgeted = false;
            while items.len() > 1
                && serde_json::to_vec(&items).map(|v| v.len()).unwrap_or(0)
                    > MAX_RESPONSE_BYTES
            {
                items.pop();
                byte_budgeted = true;
            }
            if byte_budgeted {
                truncated_by.push("byte_budget");
                warnings.push(json!({
                    "code": "TRUNCATED",
                    "message": format!(
                        "result trimmed to stay under {MAX_RESPONSE_BYTES}-byte server cap; narrow via scope=, file_pattern=, or a tighter query."
                    ),
                }));
            }

            let meta_truncated = !truncated_by.is_empty();
            let mut data = json!({
                "items": items,
                "total_matches": total_matches,
                "total_files_with_matches": total_files_with_matches,
            });
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
