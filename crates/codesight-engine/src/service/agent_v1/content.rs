//! Read symbol/file/docs content.

use serde_json::{Value, json};

use crate::config::defaults::TOKEN_HEAVY_READ_BYTES;
use crate::service::CodeService;
use crate::service::agent_v1::args::{ContentRef, ReadContentArgs};
use crate::service::agent_v1::util::{err_item, resolve_symbol_id};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::args::{FileContentArgs, GetSectionArgs};
use crate::service::savings::{MetaBuilder, RetrievalClass};

/// Strip unwanted fields and inject `ref` block into an item.
fn strip_and_tag(
    obj: &mut serde_json::Map<String, Value>,
    want_body: bool,
    want_hash: bool,
    ref_val: Value,
    body_key: &str,
) {
    if !want_body {
        obj.remove(body_key);
    }
    if !want_hash {
        obj.remove("content_hash");
    }
    obj.insert("ref".to_string(), ref_val);
}

/// Shared params threaded through each read_*_item helper.
struct ReadOpts<'a> {
    sid: &'a str,
    repo_path: Option<&'a str>,
    verify: bool,
    context_lines: usize,
    want_body: bool,
    want_hash: bool,
}

use codesight_storage::storage::IndexStore;

impl CodeService {
    fn read_symbol_item(
        &self,
        pref: &ContentRef,
        store: &IndexStore,
        opts: &ReadOpts<'_>,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<Value> {
        let resolved_id = resolve_symbol_id(
            store,
            pref.stable_id.as_deref(),
            pref.id.as_deref(),
            pref.path.as_deref(),
            pref.start_line,
            pref.symbol_name.as_deref(),
        )?;
        let Some(ref sym_id) = resolved_id else {
            let msg = match (pref.symbol_name.as_deref(), pref.path.as_deref()) {
                (Some(name), Some(path)) => format!(
                    "no symbol named '{name}' in {path} — try inspect_structure view=file_outline file_path=\"{path}\""
                ),
                (Some(name), None) => format!(
                    "symbol name '{name}' is ambiguous or not indexed — pass path + symbol_name or a stable_id"
                ),
                _ => "symbol ref needs id/stable_id, path + start_line, or path + symbol_name that resolves to an indexed symbol".to_string(),
            };
            return Ok(err_item("MISSING_REF", &msg));
        };
        *raw_bytes += store
            .symbol_metadata(sym_id)?
            .map(|m| m.file_size_bytes)
            .unwrap_or(0);
        match self.retrieve_symbol(store, sym_id, opts.verify, opts.context_lines) {
            Ok(sym) => {
                let mut o = serde_json::to_value(&sym)?;
                if let Some(obj) = o.as_object_mut() {
                    strip_and_tag(
                        obj,
                        opts.want_body,
                        opts.want_hash,
                        json!({
                            "id": sym_id, "stable_id": sym_id, "source_id": opts.sid,
                            "domain": "code", "kind": "symbol", "path": sym.file,
                        }),
                        "source",
                    );
                }
                Ok(o)
            }
            Err(e) => Ok(err_item(e.code, &e.message)),
        }
    }

    fn read_section_item(
        &self,
        sec_id: &str,
        opts: &ReadOpts<'_>,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<Value> {
        let store = self
            .ensure_doc_store(opts.sid, opts.repo_path)
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
        *raw_bytes += store.section_file_size(sec_id)?.unwrap_or(0);
        let get = GetSectionArgs {
            source_id: opts.sid.to_string(),
            section_id: Some(sec_id.to_string()),
            id: None,
            verify: opts.verify,
            repo_path: opts.repo_path.map(|s| s.to_string()),
        };
        match self.get_section(get) {
            Ok(v) => {
                if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    return Ok(err_item(
                        v["error"]["code"].as_str().unwrap_or("ERROR"),
                        v["error"]["message"]
                            .as_str()
                            .unwrap_or("get_section failed"),
                    ));
                }
                let mut o = v["data"].clone();
                if let Some(obj) = o.as_object_mut() {
                    let path = obj.get("file").cloned().unwrap_or(Value::Null);
                    strip_and_tag(
                        obj,
                        opts.want_body,
                        opts.want_hash,
                        json!({
                            "id": sec_id, "stable_id": sec_id, "source_id": opts.sid,
                            "domain": "docs", "kind": "section", "path": path,
                        }),
                        "content",
                    );
                }
                Ok(o)
            }
            Err(e) => Ok(err_item("INTERNAL", &e.to_string())),
        }
    }

    fn read_file_item(
        &self,
        pref: &ContentRef,
        file_path: &str,
        opts: &ReadOpts<'_>,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<Value> {
        let fc = FileContentArgs {
            source_id: opts.sid.to_string(),
            file_path: Some(file_path.to_string()),
            path: None,
            start_line: pref.start_line,
            end_line: pref.end_line,
            repo_path: opts.repo_path.map(|s| s.to_string()),
        };
        match self.get_file_content(fc) {
            Ok(v) => {
                if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    return Ok(err_item(
                        v["error"]["code"].as_str().unwrap_or("ERROR"),
                        v["error"]["message"].as_str().unwrap_or("read failed"),
                    ));
                }
                let mut data = v["data"].clone();
                if let Some(content_str) = data.get("content").and_then(|v| v.as_str()) {
                    *raw_bytes = raw_bytes.saturating_add(content_str.len() as u64);
                }
                if let Some(obj) = data.as_object_mut() {
                    if !opts.want_body {
                        obj.remove("content");
                    }
                    obj.insert("ref".to_string(), json!({
                        "source_id": opts.sid, "domain": "code", "kind": "file", "path": file_path,
                    }));
                }
                Ok(data)
            }
            Err(e) => Ok(err_item("INTERNAL", &e.to_string())),
        }
    }

    pub(in crate::service) fn read_content(&self, v: Value) -> anyhow::Result<Value> {
        let args: ReadContentArgs = match ValidatedArgs::from_validated(v) {
            Ok(a) => a.into_inner(),
            Err(err) => return self.error_response(&MetaBuilder::new(0), err),
        };
        // FEEDBACK.md §5b: caller opted in to wait-until-fresh semantics.
        // Bounded so a stuck reindex can't hang the client forever.
        if args.wait_for_fresh.unwrap_or(false) {
            let timeout_ms = args.max_wait_ms.unwrap_or(2000).min(30_000);
            let _ = self
                .reindex_states()
                .wait_for_fresh_result(&args.source_id, Some(timeout_ms));
        }
        let context_lines = (args.context_lines.max(0) as usize).min(50);
        let include = args.include.unwrap_or_else(|| vec!["body".to_string()]);
        let want_body = include.iter().any(|x| x == "body");
        let want_hash = include.iter().any(|x| x == "hash");
        let repo_path = args.repo_path.as_deref();
        let default_repo_store = self.ensure_repo_store(&args.source_id, repo_path).ok();

        // FEEDBACK.md §3a: soft byte budget. When exceeded, later refs stub
        // out with a structured suggestion instead of returning a JSON-escaped
        // unreadable preview at the client. Uses BYTES_PER_TOKEN to keep the
        // conversion consistent with the rest of the engine.
        let bytes_budget: Option<u64> = args.max_tokens.map(|max_tokens| {
            (max_tokens as u64).saturating_mul(codesight_storage::config::BYTES_PER_TOKEN)
        });

        let mut items = Vec::new();
        let mut raw_bytes_total = 0u64;
        let mut warnings = Vec::new();
        let mut scopes = ReadScopeCounters::default();
        let mut returned_bytes = 0u64;
        let mut warned_full_file_reads = 0u64;
        let mut budget_exhausted = false;

        for pref in &args.refs {
            if budget_exhausted {
                items.push(oversized_ref_stub(pref));
                warnings.push(token_budget_warning(pref));
                continue;
            }
            let sid = pref.source_id.as_deref().unwrap_or(&args.source_id);
            let repo_store_for_ref = if sid == args.source_id {
                default_repo_store.clone()
            } else {
                self.ensure_repo_store(sid, repo_path).ok()
            };
            let opts = ReadOpts {
                sid,
                repo_path,
                verify: args.verify,
                context_lines,
                want_body,
                want_hash,
            };
            match self.read_one_ref(
                pref,
                &opts,
                repo_store_for_ref.as_ref(),
                &mut scopes,
                &mut raw_bytes_total,
            )? {
                RefReadResult::Err(e) => items.push(e),
                RefReadResult::Ok { mut item, scope } => {
                    let item_bytes = extract_item_bytes(&item);
                    returned_bytes += item_bytes;
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("read_scope".to_string(), json!(scope));
                    }
                    if scope == "file_full" && item_bytes > TOKEN_HEAVY_READ_BYTES {
                        warnings.push(token_heavy_read_warning(pref, item_bytes));
                        warned_full_file_reads += 1;
                    }
                    items.push(item);
                    if let Some(budget) = bytes_budget
                        && returned_bytes > budget
                    {
                        budget_exhausted = true;
                    }
                }
            }
        }

        // FEEDBACK.md §3b: classify the retrieval so `tokens_saved: 0` on a
        // whole-file read doesn't read as misuse. Pick the dominant scope —
        // any non-file-full ref makes the overall class targeted.
        let class = if scopes.symbol > 0 || scopes.section > 0 || scopes.file_range > 0 {
            RetrievalClass::TargetedRange
        } else {
            RetrievalClass::WholeFileRead
        };
        let timer = MetaBuilder::new(raw_bytes_total.max(1)).with_class(class);
        let extra = build_read_meta_extras(
            &scopes,
            returned_bytes,
            warned_full_file_reads,
            args.verify,
            want_hash,
            budget_exhausted,
        );
        self.success_response_with_meta_extra(&timer, json!({ "items": items }), warnings, extra)
    }

    fn read_one_ref(
        &self,
        pref: &ContentRef,
        opts: &ReadOpts<'_>,
        repo_store: Option<&IndexStore>,
        scopes: &mut ReadScopeCounters,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<RefReadResult> {
        let id = pref.stable_id.clone().or(pref.id.clone());
        match pref.kind.to_ascii_lowercase().as_str() {
            "symbol" => {
                scopes.symbol += 1;
                let Some(store) = repo_store else {
                    return Ok(RefReadResult::Err(err_item(
                        "NOT_INDEXED",
                        &format!("Code source '{}' has not been indexed.", opts.sid),
                    )));
                };
                let item = self.read_symbol_item(pref, store, opts, raw_bytes)?;
                Ok(RefReadResult::Ok {
                    item,
                    scope: "symbol",
                })
            }
            "section" => {
                scopes.section += 1;
                let Some(sec_id) = id.as_deref() else {
                    return Ok(RefReadResult::Err(err_item(
                        "MISSING_REF",
                        "section ref needs id/stable_id",
                    )));
                };
                let item = self.read_section_item(sec_id, opts, raw_bytes)?;
                Ok(RefReadResult::Ok {
                    item,
                    scope: "section",
                })
            }
            "file" => {
                let Some(file_path) = pref.path.as_deref().or(id.as_deref()) else {
                    return Ok(RefReadResult::Err(err_item(
                        "MISSING_REF",
                        "file ref needs path or id",
                    )));
                };
                let scope = if pref.start_line.is_some() && pref.end_line.is_some() {
                    scopes.file_range += 1;
                    "file_range"
                } else {
                    scopes.file_full += 1;
                    "file_full"
                };
                let item = self.read_file_item(pref, file_path, opts, raw_bytes)?;
                Ok(RefReadResult::Ok { item, scope })
            }
            other => Ok(RefReadResult::Err(err_item(
                "UNSUPPORTED_KIND",
                &format!("kind '{other}' is not supported"),
            ))),
        }
    }
}

#[derive(Default)]
struct ReadScopeCounters {
    symbol: u64,
    section: u64,
    file_range: u64,
    file_full: u64,
}

enum RefReadResult {
    Ok {
        item: Value,
        scope: &'static str,
    },
    Err(Value),
}

fn extract_item_bytes(item: &Value) -> u64 {
    item.get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.len() as u64)
        .or_else(|| {
            item.get("source")
                .and_then(|v| v.as_str())
                .map(|s| s.len() as u64)
        })
        .unwrap_or(0)
}

/// Stub emitted in place of an unread ref when the running byte budget has
/// already been exhausted. Carries a concrete alternative tool + args so the
/// agent can retry without guessing what a "token_budget_exceeded" means.
fn oversized_ref_stub(pref: &ContentRef) -> Value {
    let path = pref
        .path
        .as_deref()
        .or(pref.stable_id.as_deref())
        .or(pref.id.as_deref())
        .unwrap_or("unknown");
    json!({
        "ref": {
            "kind": pref.kind,
            "path": path,
            "source_id": pref.source_id,
        },
        "truncated": true,
        "reason": "token_budget_exceeded",
        "suggestion": {
            "tool": "inspect_structure",
            "args": {"view": "file_outline", "file_path": path},
        },
    })
}

/// Per-trimmed-ref warning (FEEDBACK.md §3a). One per stubbed ref so the
/// agent can see exactly which inputs it paid for and which it needs to
/// re-fetch via a narrower call.
fn token_budget_warning(pref: &ContentRef) -> Value {
    let path = pref
        .path
        .as_deref()
        .or(pref.stable_id.as_deref())
        .or(pref.id.as_deref())
        .unwrap_or("unknown");
    json!({
        "code": "TOKEN_HEAVY_READ",
        "message": format!(
            "Ref '{}' was stubbed because the running byte budget was exhausted. Retry \
             with inspect_structure view=file_outline, or read_content with \
             start_line/end_line.",
            path,
        ),
        "detail": {
            "reason": "token_budget_exceeded",
            "path": path,
        },
    })
}

fn token_heavy_read_warning(pref: &ContentRef, item_bytes: u64) -> Value {
    let path_str = pref
        .path
        .as_deref()
        .or(pref.stable_id.as_deref())
        .or(pref.id.as_deref())
        .unwrap_or("unknown");
    json!({
        "code": "TOKEN_HEAVY_READ",
        "message": format!(
            "Whole-file read returned {} bytes. Token-efficient alternatives: \
             inspect_structure view=file_outline, search_catalog with include_body=true, \
             or read_content with start_line/end_line.",
            item_bytes
        ),
        "detail": {
            "read_scope": "file_full",
            "path": path_str,
            "returned_bytes": item_bytes,
            "threshold_bytes": TOKEN_HEAVY_READ_BYTES
        }
    })
}

fn build_read_meta_extras(
    scopes: &ReadScopeCounters,
    returned_bytes: u64,
    warned_full_file_reads: u64,
    verify: bool,
    want_hash: bool,
    budget_exhausted: bool,
) -> Vec<(String, Value)> {
    let mut extra: Vec<(String, Value)> = vec![
        (
            "read_scopes".to_string(),
            json!({
                "symbol": scopes.symbol,
                "section": scopes.section,
                "file_range": scopes.file_range,
                "file_full": scopes.file_full
            }),
        ),
        ("returned_bytes".to_string(), json!(returned_bytes)),
        (
            "warned_full_file_reads".to_string(),
            json!(warned_full_file_reads),
        ),
    ];

    // FEEDBACK.md §3a: strip `next_step_hint` once the response is already
    // over budget — it's duplicative and costs tokens we don't have.
    if budget_exhausted {
        extra.push(("truncated".to_string(), json!(true)));
        extra.push(("truncated_by".to_string(), json!(["token_budget"])));
        return extra;
    }

    if verify || want_hash {
        extra.push((
            "next_step_hint".to_string(),
            json!({
                "tool": "read_content",
                "reason": "This response includes verification/hash-backed bytes that some clients reuse for edit-safety checks.",
                "when": "before editing the same file only if your client requires an explicit native-read acknowledgement beyond MCP retrieval",
            }),
        ));
    } else if warned_full_file_reads > 0 {
        extra.push((
            "next_step_hint".to_string(),
            json!({
                "tool": "inspect_structure",
                "reason": "This response included whole-file reads that could be replaced with targeted retrieval.",
                "alternatives": [
                    {
                        "tool": "inspect_structure",
                        "args": {"view": "file_outline"},
                        "when": "to see what's in a file without reading all of it"
                    },
                    {
                        "tool": "search_catalog",
                        "args": {"query_mode": "keyword", "include_body": true},
                        "when": "to find and read a specific symbol or pattern"
                    },
                    {
                        "tool": "read_content",
                        "args": {"refs": [{"kind": "file", "start_line": "N", "end_line": "M"}]},
                        "when": "to read a bounded range instead of the whole file"
                    }
                ]
            }),
        ));
    }
    extra
}
