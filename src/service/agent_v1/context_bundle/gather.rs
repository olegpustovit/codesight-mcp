use std::fs;

use serde_json::{Value, json};

use crate::core::symbols::SymbolKind;
use codesight_imports::imports::context_bundle_import_lines;
use crate::service::CodeService;
use crate::service::agent_v1::args::ContentRef;
use crate::service::agent_v1::util::resolve_symbol_id;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use crate::storage::IndexStore;

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
        let mut rows = Vec::new();
        for pref in sym_refs {
            let sym_id = match resolve_symbol_id(
                store,
                pref.stable_id.as_deref(),
                pref.id.as_deref(),
                pref.path.as_deref(),
                pref.start_line,
            ) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    return Ok(Err(self.error_response(
                        timer,
                        ToolError {
                            category: ToolErrorCategory::InvalidArgument,
                            code: "INVALID_REQUEST",
                            message: "symbol ref needs id/stable_id, or path + start_line."
                                .to_string(),
                            detail: json!({}),
                        },
                    )?));
                }
                Err(e) => return Ok(Err(self.error_response(timer, index_error(e))?)),
            };

            let sym = match self.retrieve_symbol(store, &sym_id, false, 0) {
                Ok(s) => s,
                Err(e) => return Ok(Err(self.error_response(timer, e)?)),
            };

            let name_result = try_store!(self, timer, store.symbol_names_by_id(&sym_id));
            let name = name_result.map(|(n, _)| n).unwrap_or_else(|| {
                sym_id
                    .rsplit("::")
                    .next()
                    .unwrap_or(&sym_id)
                    .split('#')
                    .next()
                    .unwrap_or(&sym_id)
                    .to_string()
            });

            let signature =
                try_store!(self, timer, store.symbol_signature_by_id(&sym_id)).unwrap_or_default();

            let lang = try_store!(self, timer, store.file_language(&sym.file))
                .unwrap_or_else(|| "python".to_string());

            let root = try_store!(self, timer, store.root_path());
            let file_abs = root.join(&sym.file);
            let file_content = match fs::read_to_string(&file_abs) {
                Ok(c) => c,
                Err(_) => {
                    return Ok(Err(self.error_response(
                        timer,
                        ToolError {
                            category: ToolErrorCategory::NotFound,
                            code: "NOT_FOUND",
                            message: format!("File '{}' is missing from disk.", sym.file),
                            detail: json!({ "file": sym.file }),
                        },
                    )?));
                }
            };
            let imports = context_bundle_import_lines(&file_content, &sym.file, &lang);

            rows.push(BundleRow {
                sym_id,
                name,
                signature,
                kind: sym.kind,
                file: sym.file,
                imports,
                source: sym.source,
            });
        }
        Ok(Ok(rows))
    }
}
