//! Read symbol/file/docs content.

use serde_json::{Value, json};

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

impl CodeService {
    fn read_symbol_item(
        &self,
        pref: &ContentRef,
        opts: &ReadOpts<'_>,
        raw_bytes: &mut u64,
    ) -> anyhow::Result<Value> {
        let store = self
            .ensure_repo_store(opts.sid, opts.repo_path)
            .map_err(|e| anyhow::anyhow!("{}", e.message))?;
        let resolved_id = resolve_symbol_id(
            &store,
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
        match self.retrieve_symbol(&store, sym_id, opts.verify, opts.context_lines) {
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
        let store_check = self.ensure_repo_store(opts.sid, opts.repo_path);
        let fc = FileContentArgs {
            source_id: opts.sid.to_string(),
            file_path: Some(file_path.to_string()),
            path: None,
            start_line: pref.start_line,
            end_line: pref.end_line,
            repo_path: opts.repo_path.map(|s| s.to_string()),
        };
        match (store_check, self.get_file_content(fc)) {
            (Ok(_), Ok(v)) => {
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
            (Err(e), _) => Ok(err_item(e.code, &e.message)),
            (_, Err(e)) => Ok(err_item("INTERNAL", &e.to_string())),
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

        let mut items = Vec::new();
        let mut raw_bytes_total = 0u64;

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

            let item = match pref.kind.to_ascii_lowercase().as_str() {
                "symbol" => self.read_symbol_item(pref, &opts, &mut raw_bytes_total)?,
                "section" => {
                    let Some(ref sec_id) = id else {
                        items.push(err_item("MISSING_REF", "section ref needs id/stable_id"));
                        continue;
                    };
                    self.read_section_item(sec_id, &opts, &mut raw_bytes_total)?
                }
                "file" => {
                    let Some(file_path) = pref.path.as_deref().or(id.as_deref()) else {
                        items.push(err_item("MISSING_REF", "file ref needs path or id"));
                        continue;
                    };
                    self.read_file_item(pref, file_path, &opts, &mut raw_bytes_total)?
                }
                other => err_item(
                    "UNSUPPORTED_KIND",
                    &format!("kind '{}' is not supported", other),
                ),
            };
            items.push(item);
        }

        let timer = MetaBuilder::new(raw_bytes_total.max(1));
        self.success_response_with_meta_extra(
            &timer,
            json!({ "items": items }),
            Vec::new(),
            [(
                "tip".to_string(),
                json!("Use the Read tool if you plan to edit these files — the Claude Code harness tracks read state separately."),
            )],
        )
    }
}
