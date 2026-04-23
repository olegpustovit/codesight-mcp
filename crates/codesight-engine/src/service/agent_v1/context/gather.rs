use std::fs;

use serde_json::{Value, json};

use codesight_core::symbols::SymbolKind;
use crate::service::CodeService;
use crate::service::agent_v1::args::ContentRef;
use crate::service::agent_v1::util::resolve_symbol_id;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::IndexStore;
use codesight_language_support::imports_orchestration::context_bundle_import_lines;

#[derive(Clone)]
pub(super) struct BundleRow {
    pub sym_id: String,
    pub name: String,
    pub signature: String,
    pub kind: SymbolKind,
    pub file: String,
    pub imports: Vec<String>,
    pub source: String,
}

macro_rules! try_store {
    ($self:expr, $timer:expr, $expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(e) => return Ok(Err($self.error_response($timer, index_error(e))?)),
        }
    };
}

impl CodeService {
    pub(super) fn gather_bundle_rows(
        &self,
        store: &IndexStore,
        sym_refs: &[&ContentRef],
        timer: &MetaBuilder,
    ) -> anyhow::Result<Result<Vec<BundleRow>, Value>> {
        // Pass 1 — resolve each ref to an id up front. `resolve_symbol_id`
        // may touch the DB (path+line fallback) but at most once per ref
        // regardless of how we batch the rest.
        let mut ids: Vec<String> = Vec::with_capacity(sym_refs.len());
        for pref in sym_refs {
            let id = match resolve_symbol_id(
                store,
                pref.stable_id.as_deref(),
                pref.id.as_deref(),
                pref.path.as_deref(),
                pref.start_line,
                pref.symbol_name.as_deref(),
            ) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    let message = match (pref.symbol_name.as_deref(), pref.path.as_deref()) {
                        (Some(name), Some(path)) => format!(
                            "no symbol named '{name}' in {path} — try inspect_structure view=file_outline file_path=\"{path}\""
                        ),
                        (Some(name), None) => format!(
                            "symbol name '{name}' is ambiguous or not indexed — pass path + symbol_name or a stable_id"
                        ),
                        _ => "symbol ref needs id/stable_id, path + start_line, or path + symbol_name.".to_string(),
                    };
                    return Ok(Err(self.error_response(
                        timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message,
                            detail: json!({
                                "symbol_name": pref.symbol_name,
                                "path": pref.path,
                            }),
                        },
                    )?));
                }
                Err(e) => return Ok(Err(self.error_response(timer, index_error(e))?)),
            };
            ids.push(id);
        }

        // Pass 2 — batch-fetch everything symbol-keyed in one DB trip each.
        // Replaces 4 single-key DB calls per ref (symbol_record + names +
        // signature + file_language) plus one `root_path()` per iteration.
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let enriched = try_store!(self, timer, store.symbol_enriched_batch(&id_refs));
        let records = try_store!(self, timer, store.symbol_records_batch(&id_refs));
        let root = try_store!(self, timer, store.root_path());

        // Pass 3 — assemble each row. File disk reads are inherently
        // per-ref (to materialize imports) but dedup across refs that
        // share the same file so large bundles don't re-read.
        let mut rows = Vec::with_capacity(sym_refs.len());
        let mut file_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for sym_id in ids {
            let Some(record) = records.get(&sym_id) else {
                return Ok(Err(self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{sym_id}' not found in index."),
                        detail: json!({ "id": sym_id }),
                    },
                )?));
            };
            let meta = enriched.get(&sym_id);
            let name = meta.map(|m| m.name.clone()).unwrap_or_else(|| {
                sym_id
                    .rsplit("::")
                    .next()
                    .unwrap_or(&sym_id)
                    .split('#')
                    .next()
                    .unwrap_or(&sym_id)
                    .to_string()
            });
            let signature = meta.map(|m| m.signature.clone()).unwrap_or_default();

            let file_content = if let Some(cached) = file_cache.get(&record.file_path) {
                cached.clone()
            } else {
                let file_abs = root.join(&record.file_path);
                let content = match fs::read_to_string(&file_abs) {
                    Ok(c) => c,
                    Err(_) => {
                        return Ok(Err(self.error_response(
                            timer,
                            ToolError {
                                category: ToolErrorCategory::NotFound,
                                code: "NOT_FOUND",
                                message: format!(
                                    "File '{}' is missing from disk.",
                                    record.file_path
                                ),
                                detail: json!({ "file": record.file_path }),
                            },
                        )?));
                    }
                };
                file_cache.insert(record.file_path.clone(), content.clone());
                content
            };
            let imports = context_bundle_import_lines(&file_content, &record.file_path);

            rows.push(BundleRow {
                sym_id: record.id.clone(),
                name,
                signature,
                kind: record.kind,
                file: record.file_path.clone(),
                imports,
                source: record.stored_source.clone(),
            });
        }
        Ok(Ok(rows))
    }
}
