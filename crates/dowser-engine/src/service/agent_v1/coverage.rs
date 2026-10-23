//! Index membership and capability boundaries shared by public navigation tools.
use super::warnings;
use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use dowser_storage::storage::IndexStore;
use serde_json::{Value, json};

impl CodeService {
    pub(super) fn import_graph_path<'a>(
        &self,
        store: &IndexStore,
        args: &'a InspectGraphArgs,
    ) -> Result<&'a str, ToolError> {
        let path = args.file_path.as_deref().ok_or_else(|| {
            ToolError::new(
                ToolErrorCategory::InvalidArgument,
                "INVALID_REQUEST",
                format!("{} mode requires file_path.", args.graph),
                json!({}),
            )
        })?;
        let language = store.file_language(path).map_err(index_error)?.ok_or_else(|| ToolError::new(
            ToolErrorCategory::NotFound, "FILE_NOT_INDEXED",
            "Requested file is outside the index; unsupported or excluded files cannot be treated as having no imports.",
            json!({"path":path,"source_id":args.source_id})))?;
        if args.graph == "imports"
            && !dowser_language_support::supports_import_extraction(&language)
        {
            return Err(ToolError::new(
                ToolErrorCategory::InvalidArgument,
                "IMPORT_EXTRACTION_UNSUPPORTED",
                "This indexed language has no import extractor; absence of import results would be inconclusive.",
                json!({"path":path,"language":language}),
            ));
        }
        Ok(path)
    }

    pub(super) fn annotate_indexed_scope(&self, source_id: &str, response: &mut Value) {
        if response["ok"] != true {
            return;
        }
        let supported = dowser_language_support::builtin_registry().supported_languages();
        response["data"]["coverage"] = json!({
            "scope":"indexed_files_only", "repository_coverage_complete":false,
            "index_file_count":self.open_repo_store(source_id).ok().and_then(|store|store.file_count().ok()),
            "omitted_file_count":null, "builtin_languages":supported,
        });
        if response["_meta"]["confidence"] == "safe" {
            response["_meta"]["confidence"] = json!("partial");
        }
        response["warnings"].as_array_mut().unwrap().push(warnings::warning(warnings::INDEX_SCOPE_ONLY,
            "Results cover indexed files, not a complete repository audit. Unsupported, ignored or unindexed files may be absent."));
    }
}
