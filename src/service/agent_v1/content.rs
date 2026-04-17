//! Read symbol/file/docs content.

use serde_json::{Value, json};

use crate::config::defaults::TOKEN_HEAVY_READ_BYTES;
use crate::service::CodeService;
use crate::service::agent_v1::args::{ContentRef, ReadContentArgs};
use crate::service::agent_v1::util::{err_item, resolve_symbol_id};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::args::{FileContentArgs, GetSectionArgs};
use crate::service::savings::MetaBuilder;

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

use crate::storage::IndexStore;

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
        )?;
        let Some(ref sym_id) = resolved_id else {
            return Ok(err_item(
                "MISSING_REF",
                "symbol ref needs id/stable_id, or path + start_line that resolves to an indexed symbol",
            ));
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
        let context_lines = (args.context_lines.max(0) as usize).min(50);
        let include = args.include.unwrap_or_else(|| vec!["body".to_string()]);
        let want_body = include.iter().any(|x| x == "body");
        let want_hash = include.iter().any(|x| x == "hash");
        let repo_path = args.repo_path.as_deref();

        let heavy_threshold = TOKEN_HEAVY_READ_BYTES;

        // Resolve the default repo store ONCE before the loop. This avoids
        // repeated staleness checks + potential reindexes per ref.
        let default_repo_store = self.ensure_repo_store(&args.source_id, repo_path).ok();

        let mut items = Vec::new();
        let mut raw_bytes_total = 0u64;
        let mut warnings = Vec::new();

        // Read scope counters.
        let mut scope_symbol = 0u64;
        let mut scope_section = 0u64;
        let mut scope_file_range = 0u64;
        let mut scope_file_full = 0u64;
        let mut warned_full_file_reads = 0u64;
        let mut returned_bytes = 0u64;

        for pref in &args.refs {
            let sid = pref.source_id.as_deref().unwrap_or(&args.source_id);
            let id = pref.stable_id.clone().or(pref.id.clone());
            let opts = ReadOpts {
                sid,
                repo_path,
                verify: args.verify,
                context_lines,
                want_body,
                want_hash,
            };

            // Reuse pre-resolved store when source_id matches; resolve on
            // demand only for cross-source refs.
            let repo_store_for_ref = if sid == args.source_id {
                default_repo_store.clone()
            } else {
                self.ensure_repo_store(sid, repo_path).ok()
            };

            let (mut item, scope) = match pref.kind.to_ascii_lowercase().as_str() {
                "symbol" => {
                    scope_symbol += 1;
                    let Some(ref store) = repo_store_for_ref else {
                        items.push(err_item(
                            "NOT_INDEXED",
                            &format!("Code source '{sid}' has not been indexed."),
                        ));
                        continue;
                    };
                    (
                        self.read_symbol_item(pref, store, &opts, &mut raw_bytes_total)?,
                        "symbol",
                    )
                }
                "section" => {
                    scope_section += 1;
                    let Some(ref sec_id) = id else {
                        items.push(err_item("MISSING_REF", "section ref needs id/stable_id"));
                        continue;
                    };
                    (
                        self.read_section_item(sec_id, &opts, &mut raw_bytes_total)?,
                        "section",
                    )
                }
                "file" => {
                    let Some(file_path) = pref.path.as_deref().or(id.as_deref()) else {
                        items.push(err_item("MISSING_REF", "file ref needs path or id"));
                        continue;
                    };
                    let is_bounded = pref.start_line.is_some() && pref.end_line.is_some();
                    if is_bounded {
                        scope_file_range += 1;
                    } else {
                        scope_file_full += 1;
                    }
                    let scope_label = if is_bounded {
                        "file_range"
                    } else {
                        "file_full"
                    };
                    (
                        self.read_file_item(pref, file_path, &opts, &mut raw_bytes_total)?,
                        scope_label,
                    )
                }
                other => {
                    items.push(err_item(
                        "UNSUPPORTED_KIND",
                        &format!("kind '{}' is not supported", other),
                    ));
                    continue;
                }
            };

            // Track returned bytes per item.
            let item_bytes = item
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len() as u64)
                .or_else(|| {
                    item.get("source")
                        .and_then(|v| v.as_str())
                        .map(|s| s.len() as u64)
                })
                .unwrap_or(0);
            returned_bytes += item_bytes;

            // Inject read_scope into item.
            if let Some(obj) = item.as_object_mut() {
                obj.insert("read_scope".to_string(), json!(scope));
            }

            // TOKEN_HEAVY_READ: only on file_full above threshold.
            if scope == "file_full" && item_bytes > heavy_threshold {
                let path_str = pref.path.as_deref().or(id.as_deref()).unwrap_or("unknown");
                warnings.push(json!({
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
                        "threshold_bytes": heavy_threshold
                    }
                }));
                warned_full_file_reads += 1;
            }

            items.push(item);
        }

        let timer = MetaBuilder::new(raw_bytes_total.max(1));
        let mut extra: Vec<(String, Value)> = Vec::new();

        // Read scope counters in _meta.
        extra.push((
            "read_scopes".to_string(),
            json!({
                "symbol": scope_symbol,
                "section": scope_section,
                "file_range": scope_file_range,
                "file_full": scope_file_full
            }),
        ));
        extra.push(("returned_bytes".to_string(), json!(returned_bytes)));
        extra.push((
            "warned_full_file_reads".to_string(),
            json!(warned_full_file_reads),
        ));

        if args.verify || want_hash {
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

        self.success_response_with_meta_extra(&timer, json!({ "items": items }), warnings, extra)
    }
}
