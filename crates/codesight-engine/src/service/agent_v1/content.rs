//! Read symbol/file/docs content.

use serde_json::{Value, json};

use crate::config::defaults::TOKEN_HEAVY_READ_BYTES;
use crate::service::CodeService;
use crate::service::agent_v1::args::{ContentRef, ReadContentArgs};
use crate::service::agent_v1::util::{err_item, resolve_symbol_id};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::args::{FileContentArgs, GetSectionArgs};
use crate::service::savings::{Confidence, MetaBuilder, RetrievalClass};

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
        repo_store: Option<&IndexStore>,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_doc_store(opts.sid, opts.repo_path) {
            Ok(s) => s,
            // Tier 3.1: doc index unavailable → parse the file directly using
            // codesight_retrieval's section extractor. The section ref form
            // already encodes the path, so we don't need a doc index for this.
            Err(e) if e.code == "NOT_INDEXED" => {
                return self.read_section_from_file_fallback(sec_id, repo_store, raw_bytes, opts);
            }
            Err(e) => return Err(anyhow::anyhow!("{}", e.message)),
        };
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

    /// Tier 3.1: parse a `section:path::heading` ref directly off disk when
    /// no doc index exists. Reuses
    /// [`codesight_retrieval::sections::extract_sections`] so the heading
    /// resolution rules match what the indexed path would have done.
    fn read_section_from_file_fallback(
        &self,
        sec_id: &str,
        repo_store: Option<&IndexStore>,
        raw_bytes: &mut u64,
        opts: &ReadOpts<'_>,
    ) -> anyhow::Result<Value> {
        let Some((path, heading)) = sec_id.split_once("::") else {
            return Ok(err_item(
                "MISSING_REF",
                "section id must be 'path::heading'",
            ));
        };
        let Some(store) = repo_store else {
            return Ok(err_item(
                "NOT_INDEXED",
                &format!(
                    "neither code nor doc source '{}' is indexed; cannot resolve `section:{path}::{heading}`.",
                    opts.sid,
                ),
            ));
        };
        let root = match store.root_path() {
            Ok(r) => r,
            Err(e) => return Ok(err_item("INTERNAL", &format!("root_path: {e}"))),
        };
        let abs = root.join(path);
        let source = match std::fs::read_to_string(&abs) {
            Ok(s) => s,
            Err(e) => {
                return Ok(err_item("READ_FAILED", &format!("read {path}: {e}")));
            }
        };
        *raw_bytes += source.len() as u64;
        let sections = codesight_retrieval::sections::extract_sections(path, &source);
        let target = sections.into_iter().find(|s| {
            s.heading.eq_ignore_ascii_case(heading) || s.heading_path.eq_ignore_ascii_case(heading)
        });
        let Some(section) = target else {
            return Ok(err_item(
                crate::service::agent_v1::warnings::SECTION_NOT_FOUND,
                &format!("heading '{heading}' not found in {path}"),
            ));
        };
        let content = source
            .get(section.start_byte..section.end_byte)
            .unwrap_or("")
            .to_string();
        let mut item = json!({
            "ref": {
                "id": sec_id,
                "stable_id": sec_id,
                "source_id": opts.sid,
                "domain": "docs",
                "kind": "section",
                "path": path,
                "heading": section.heading,
                "heading_path": section.heading_path,
                "level": section.level,
                "start_line": section.start_line,
                "end_line": section.end_line,
            },
            "summary": section.summary,
            "fallback_source": "file_parse",
        });
        if opts.want_body
            && let Value::Object(obj) = &mut item
        {
            obj.insert("content".to_string(), json!(content));
        }
        Ok(item)
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
        // ROADMAP P1.1: baseline for any file read (full or range) is the
        // full containing file size — the counterfactual is "the agent would
        // have `cat`-ed the whole file". For whole-file reads the response
        // equals the baseline → tokens_saved ≈ 0 (honest). For ranges, the
        // savings reflect the bytes the agent didn't have to page through.
        if let Some(store) = self
            .ensure_repo_store(opts.sid, opts.repo_path)
            .ok()
            .as_ref()
            && let Ok(Some(bytes)) = store.file_bytes(file_path)
        {
            *raw_bytes = raw_bytes.saturating_add(bytes);
        }
        match self.get_file_content(fc) {
            Ok(v) => {
                if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                    return Ok(err_item(
                        v["error"]["code"].as_str().unwrap_or("ERROR"),
                        v["error"]["message"].as_str().unwrap_or("read failed"),
                    ));
                }
                let mut data = v["data"].clone();
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
        // Tier 2.2: per-ref errors land here, separated from `items[]` so
        // callers can iterate successes without filtering. Surfaced as
        // `data.errors[]` in the response with a `PARTIAL_BATCH` warning.
        let mut errors: Vec<Value> = Vec::new();
        let mut raw_bytes_total = 0u64;
        let mut warnings = Vec::new();
        let mut scopes = ReadScopeCounters::default();
        let mut returned_bytes = 0u64;
        let mut warned_full_file_reads = 0u64;
        let mut budget_exhausted = false;
        let mut actually_stubbed = 0u64;

        for pref in &args.refs {
            if budget_exhausted {
                items.push(oversized_ref_stub(pref));
                warnings.push(token_budget_warning(pref));
                actually_stubbed += 1;
                continue;
            }
            let sid = pref.source_id.as_deref().unwrap_or(&args.source_id);
            let repo_store_for_ref = if sid == args.source_id {
                default_repo_store.clone()
            } else {
                self.ensure_repo_store(sid, repo_path).ok()
            };

            // Pre-flight: for whole-file reads we can project the cost via
            // `file_bytes`. If the projection would blow the running byte
            // budget, replace the body with an outline-as-stub *instead* of
            // reading the file. The post-loop check below covers the
            // symbol/section/range cases where projection isn't free.
            if let Some(preflight) = try_preflight_oversized_file(
                pref,
                repo_store_for_ref.as_ref(),
                returned_bytes,
                bytes_budget,
            ) {
                items.push(preflight.stub);
                warnings.push(preflight.warning);
                actually_stubbed += 1;
                budget_exhausted = true;
                scopes.file_full += 1;
                continue;
            }

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
                RefReadResult::Err(mut e) => {
                    // Tag the failing ref so callers can correlate without
                    // re-parsing the message. `pref.path` and `pref.id` are
                    // the two stable identifiers we expose on input.
                    if let Value::Object(obj) = &mut e {
                        if let Some(p) = pref.path.as_deref() {
                            obj.insert("ref_path".to_string(), json!(p));
                        }
                        if let Some(id) = pref.stable_id.as_deref().or(pref.id.as_deref()) {
                            obj.insert("ref_id".to_string(), json!(id));
                        }
                    }
                    errors.push(e);
                }
                RefReadResult::Ok { mut item, scope } => {
                    let item_bytes = extract_item_bytes(&item);
                    returned_bytes += item_bytes;
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("read_scope".to_string(), json!(scope));
                    }
                    if scope == "file_full" && item_bytes > TOKEN_HEAVY_READ_BYTES {
                        warnings.push(token_heavy_read_warning(pref, item_bytes));
                        warned_full_file_reads += 1;
                        // In-budget but heavy: keep the data and offer the
                        // outline as a follow-up suggestion (not a substitute).
                        // Pre-flight stubbing handles the over-budget case
                        // before we reach this point, so this branch is now
                        // strictly an advisory.
                        if let Some(path) = pref.path.as_deref()
                            && let Some(store) = repo_store_for_ref.as_ref()
                            && let Ok(outline) = store.file_outline(path)
                            && !outline.is_empty()
                            && let Ok(outline_json) = serde_json::to_value(&outline)
                            && let Some(arr) = outline_json.as_array()
                        {
                            warnings.push(outline_first_hint(path, arr.clone()));
                        }
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
        // Tier 2.2: when any per-ref errored, demote confidence and surface
        // a top-level `PARTIAL_BATCH` warning. `data.errors[]` carries the
        // per-ref error envelopes; successful reads still land in
        // `data.items[]` so the caller never loses good work to one bad ref.
        let confidence = if errors.is_empty() {
            Confidence::Safe
        } else {
            warnings.push(crate::service::agent_v1::warnings::warning(
                crate::service::agent_v1::warnings::PARTIAL_BATCH,
                format!(
                    "{} of {} ref(s) failed; successful reads are in `data.items[]`, errors in `data.errors[]`.",
                    errors.len(),
                    args.refs.len(),
                ),
            ));
            Confidence::Partial
        };
        // ROADMAP P1.1: baseline is the honest sum of full-file sizes
        // the agent would have read without codesight; a whole-file read
        // yields savings=0 (response≈baseline), a symbol/range read yields
        // positive savings. No `.max(1)` floor — when `raw_bytes_total` is
        // legitimately 0 we want `tokens_saved: null`, not a fabricated 1.
        let timer = MetaBuilder::new(raw_bytes_total)
            .with_class(class)
            .with_confidence(confidence);
        let extra = build_read_meta_extras(
            &scopes,
            returned_bytes,
            warned_full_file_reads,
            args.verify,
            want_hash,
            actually_stubbed,
        );
        let mut data = json!({ "items": items });
        if !errors.is_empty()
            && let Value::Object(obj) = &mut data
        {
            obj.insert("errors".to_string(), json!(errors));
        }
        self.success_response_with_meta_extra(&timer, data, warnings, extra)
    }

    /// Tier 2.2: best-effort per-ref reader. Any `read_*_item` failure is
    /// caught and surfaced as `RefReadResult::Err(err_item(...))` so a single
    /// bad ref never aborts the whole batch. The outer `read_content` loop
    /// translates `Err` items into `data.errors[]` and pushes a
    /// `PARTIAL_BATCH` warning so callers see exactly which refs failed.
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
                match self.read_symbol_item(pref, store, opts, raw_bytes) {
                    Ok(item) => Ok(RefReadResult::Ok {
                        item,
                        scope: "symbol",
                    }),
                    Err(e) => Ok(RefReadResult::Err(err_item(
                        "READ_FAILED",
                        &format!("symbol read failed: {e}"),
                    ))),
                }
            }
            "section" => {
                scopes.section += 1;
                let Some(sec_id) = id.as_deref() else {
                    return Ok(RefReadResult::Err(err_item(
                        "MISSING_REF",
                        "section ref needs id/stable_id",
                    )));
                };
                match self.read_section_item(sec_id, opts, repo_store, raw_bytes) {
                    Ok(item) => Ok(RefReadResult::Ok {
                        item,
                        scope: "section",
                    }),
                    Err(e) => Ok(RefReadResult::Err(err_item(
                        "SECTION_NOT_FOUND",
                        &format!("section read failed: {e}"),
                    ))),
                }
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
                match self.read_file_item(pref, file_path, opts, raw_bytes) {
                    Ok(item) => Ok(RefReadResult::Ok { item, scope }),
                    Err(e) => Ok(RefReadResult::Err(err_item(
                        "READ_FAILED",
                        &format!("file read failed: {e}"),
                    ))),
                }
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
    Ok { item: Value, scope: &'static str },
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

struct PreflightStub {
    stub: Value,
    warning: Value,
}

/// Pre-flight projection for whole-file reads. Returns a stub + warning when
/// the file's known size would push `returned_bytes` past `bytes_budget`,
/// so callers can replace the body with an outline-as-stub *before* the read
/// happens. Skips ranged reads (we'd need line-byte offsets to project
/// honestly), non-file refs (cheap), and budget-less calls (no contract to
/// uphold). Refusing to read is the contract — augmenting an over-budget
/// response with a "you should have used outline" hint is not.
fn try_preflight_oversized_file(
    pref: &ContentRef,
    repo_store: Option<&IndexStore>,
    returned_bytes: u64,
    bytes_budget: Option<u64>,
) -> Option<PreflightStub> {
    let budget = bytes_budget?;
    if !pref.kind.eq_ignore_ascii_case("file") {
        return None;
    }
    if pref.start_line.is_some() || pref.end_line.is_some() {
        return None;
    }
    let path = pref.path.as_deref().or(pref.id.as_deref())?;
    let store = repo_store?;
    let file_size = store.file_bytes(path).ok().flatten()?;
    if returned_bytes.saturating_add(file_size) <= budget {
        return None;
    }
    // Always honor the first ref in full — even when its size alone exceeds
    // the budget. Stubbing the first ref returns nothing useful; subsequent
    // oversize refs get the running-budget stub via `oversized_ref_stub`.
    if returned_bytes == 0 {
        return None;
    }
    let outline = store.file_outline(path).ok().unwrap_or_default();
    let outline_json = serde_json::to_value(&outline).unwrap_or_else(|_| Value::Array(Vec::new()));
    let stub = json!({
        "ref": {
            "kind": pref.kind,
            "path": path,
            "source_id": pref.source_id,
        },
        "truncated": true,
        "reason": "token_budget_exceeded_preflight",
        "projected_bytes": file_size,
        "outline": outline_json,
        "suggestion": {
            "tool": "read_content",
            "args": {
                "refs": [{
                    "kind": "symbol",
                    "path": path,
                    "symbol_name": "<pick from outline>"
                }]
            },
        },
    });
    let warning = json!({
        "code": "TOKEN_BUDGET_EXCEEDED",
        "message": format!(
            "Skipped whole-file read of `{path}` ({file_size} bytes) — would have exceeded the {budget}-byte cap. Outline returned in place; retry with symbol refs."
        ),
        "detail": {
            "path": path,
            "projected_bytes": file_size,
            "budget_bytes": budget,
        },
    });
    Some(PreflightStub { stub, warning })
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

/// ROADMAP P2.3: when a caller asks for a whole-file read we can usually
/// suggest a narrower follow-up right now, rather than just hinting. The
/// warning carries the file outline inline so the agent can re-issue
/// `read_content` with specific symbol refs in one round-trip.
fn outline_first_hint(path: &str, outline: Vec<Value>) -> Value {
    json!({
        "code": "OUTLINE_FIRST_HINT",
        "message": format!(
            "Whole-file read of `{path}` — the file's outline is inline below. \
             Retry with `read_content refs=[{{kind:\"symbol\",path:\"{path}\",symbol_name:\"<pick one>\"}}]` \
             or a specific `start_line`/`end_line` range."
        ),
        "detail": {
            "path": path,
            "symbol_count": outline.len(),
            "outline": outline,
        },
    })
}

fn build_read_meta_extras(
    scopes: &ReadScopeCounters,
    returned_bytes: u64,
    warned_full_file_reads: u64,
    verify: bool,
    want_hash: bool,
    actually_stubbed: u64,
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

    // Truthful: only flag the response as truncated when at least one ref
    // was actually replaced with a stub. A budget-exhausted flag without an
    // actual stub means "the last in-budget ref happened to fit under the
    // wire" — not a truncation worth telling the caller about.
    if actually_stubbed > 0 {
        extra.push(("truncated".to_string(), json!(true)));
        extra.push(("truncated_by".to_string(), json!(["token_budget"])));
        extra.push(("stubbed_refs".to_string(), json!(actually_stubbed)));
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
