//! Read symbol/file/docs content.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::ReadContentArgs;
use crate::service::agent_v1::util::err_item;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::args::{FileContentArgs, GetSectionArgs};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn read_content(&self, v: Value) -> anyhow::Result<Value> {
        let args: ReadContentArgs = match ValidatedArgs::from_validated(v) {
            Ok(a) => a.into_inner(),
            Err(err) => return self.error_response(&MetaBuilder::new(0), err),
        };
        // Clamp context_lines: negative -> 0, excessive -> 50
        let context_lines = (args.context_lines.max(0) as usize).min(50);
        let include = args.include.unwrap_or_else(|| vec!["body".to_string()]);
        let want_body = include.iter().any(|x| x == "body");
        let want_hash = include.iter().any(|x| x == "hash");

        let mut items = Vec::new();
        let mut raw_bytes_total = 0u64;

        for pref in &args.refs {
            let sid = pref.source_id.as_deref().unwrap_or(&args.source_id);
            let id = pref.stable_id.clone().or(pref.id.clone());
            let kind = pref.kind.to_ascii_lowercase();

            let item = match kind.as_str() {
                "symbol" => {
                    let Some(ref sym_id) = id else {
                        items.push(err_item("MISSING_REF", "symbol ref needs id/stable_id"));
                        continue;
                    };
                    let store = match self.ensure_repo_store(sid, args.repo_path.as_deref()) {
                        Ok(s) => s,
                        Err(e) => {
                            items.push(err_item(e.code, &e.message));
                            continue;
                        }
                    };
                    raw_bytes_total += store
                        .symbol_metadata(sym_id)?
                        .map(|m| m.file_size_bytes)
                        .unwrap_or(0);
                    match self.retrieve_symbol(&store, sym_id, args.verify, context_lines) {
                        Ok(sym) => {
                            let mut o = serde_json::to_value(&sym)?;
                            if let Some(obj) = o.as_object_mut() {
                                if !want_body {
                                    obj.remove("source");
                                }
                                if !want_hash {
                                    obj.remove("content_hash");
                                }
                                obj.insert(
                                    "ref".to_string(),
                                    json!({
                                        "id": sym_id,
                                        "stable_id": sym_id,
                                        "source_id": sid,
                                        "domain": "code",
                                        "kind": "symbol",
                                        "path": sym.file,
                                    }),
                                );
                            }
                            o
                        }
                        Err(e) => err_item(e.code, &e.message),
                    }
                }
                "section" => {
                    let Some(ref sec_id) = id else {
                        items.push(err_item("MISSING_REF", "section ref needs id/stable_id"));
                        continue;
                    };
                    let store = match self.ensure_doc_store(sid, args.repo_path.as_deref()) {
                        Ok(s) => s,
                        Err(e) => {
                            items.push(err_item(e.code, &e.message));
                            continue;
                        }
                    };
                    raw_bytes_total += store.section_file_size(sec_id)?.unwrap_or(0);
                    let get = GetSectionArgs {
                        source_id: sid.to_string(),
                        section_id: Some(sec_id.clone()),
                        id: None,
                        verify: args.verify,
                        repo_path: args.repo_path.clone(),
                    };
                    match self.get_section(get) {
                        Ok(v) => {
                            if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                                let code = v["error"]["code"].as_str().unwrap_or("ERROR");
                                let msg = v["error"]["message"]
                                    .as_str()
                                    .unwrap_or("get_section failed");
                                err_item(code, msg)
                            } else {
                                let mut o = v["data"].clone();
                                if let Some(obj) = o.as_object_mut() {
                                    if !want_body {
                                        obj.remove("content");
                                    }
                                    if !want_hash {
                                        obj.remove("content_hash");
                                    }
                                    obj.insert(
                                        "ref".to_string(),
                                        json!({
                                            "id": sec_id,
                                            "stable_id": sec_id,
                                            "source_id": sid,
                                            "domain": "docs",
                                            "kind": "section",
                                            "path": obj.get("file").cloned().unwrap_or(Value::Null),
                                        }),
                                    );
                                }
                                o
                            }
                        }
                        Err(e) => err_item("INTERNAL", &e.to_string()),
                    }
                }
                "file" => {
                    let file_path = match pref.path.clone().or(id.clone()) {
                        Some(p) => p,
                        None => {
                            items.push(err_item("MISSING_REF", "file ref needs path or id"));
                            continue;
                        }
                    };
                    let _store_check = self.ensure_repo_store(sid, args.repo_path.as_deref());
                    let fc = FileContentArgs {
                        source_id: sid.to_string(),
                        file_path: Some(file_path.clone()),
                        path: None,
                        start_line: pref.start_line,
                        end_line: pref.end_line,
                        repo_path: args.repo_path.clone(),
                    };
                    match (_store_check, self.get_file_content(fc)) {
                        (Ok(_), Ok(v)) => {
                            if v.get("ok").and_then(|b| b.as_bool()) != Some(true) {
                                let code = v["error"]["code"].as_str().unwrap_or("ERROR");
                                let msg = v["error"]["message"].as_str().unwrap_or("read failed");
                                err_item(code, msg)
                            } else {
                                let mut data = v["data"].clone();
                                if let Some(content_str) =
                                    data.get("content").and_then(|v| v.as_str())
                                {
                                    raw_bytes_total =
                                        raw_bytes_total.saturating_add(content_str.len() as u64);
                                }
                                if let Some(obj) = data.as_object_mut() {
                                    if !want_body {
                                        obj.remove("content");
                                    }
                                    obj.insert(
                                        "ref".to_string(),
                                        json!({
                                            "source_id": sid,
                                            "domain": "code",
                                            "kind": "file",
                                            "path": file_path,
                                        }),
                                    );
                                }
                                data
                            }
                        }
                        (Err(e), _) => err_item(e.code, &e.message),
                        (_, Err(e)) => err_item("INTERNAL", &e.to_string()),
                    }
                }
                _ => err_item(
                    "UNSUPPORTED_KIND",
                    &format!("kind '{}' is not supported", kind),
                ),
            };
            items.push(item);
        }

        let timer = MetaBuilder::new(raw_bytes_total.max(1));
        self.success_response(&timer, json!({ "items": items }))
    }
}
