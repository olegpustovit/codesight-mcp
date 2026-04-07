//! List indexed sources (`catalog_sources`).

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::doc_storage::DocStore;
use crate::doc_storage::list_doc_summaries;
use crate::path_map;
use crate::savings::MetaBuilder;
use crate::service::CodeService;
use crate::service::agent_v1::args::CatalogSourcesArgs;
use crate::service::args::parse_args;
use crate::storage::{IndexStore, list_repo_summaries};

fn index_metadata_staleness_warning(indexed_at: &str) -> Option<String> {
    let dt = DateTime::parse_from_rfc3339(indexed_at).ok()?;
    let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
    let days = age.num_days();
    if days >= 7 {
        Some(format!(
            "index metadata is {days} days old; re-run ingest_sources."
        ))
    } else {
        None
    }
}

impl CodeService {
    pub(in crate::service) fn catalog_sources(&self, v: Value) -> anyhow::Result<Value> {
        let _args: CatalogSourcesArgs = parse_args(v)?;
        let timer = MetaBuilder::new(0);
        let code_rows = list_repo_summaries(&self.config.repos_dir())?;
        let doc_values = list_doc_summaries(&self.config.docs_dir())?;

        let mut sources: Vec<Value> = Vec::new();

        for m in code_rows {
            let raw_root = IndexStore::open(self.config.repo_db_path(&m.repo))
                .ok()
                .and_then(|s| s.root_path().ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let root_hint = path_map::remap(&raw_root, &self.config.path_map, false);

            let mut entry = json!({
                "source_id": m.repo,
                "display_name": m.repo,
                "root_hint": root_hint,
                "domains": ["code"],
                "status": if m.is_stale == Some(true) { "stale" } else { "indexed" },
                "capabilities": [
                    "ingest_sources", "catalog_sources", "search_catalog", "read_content",
                    "inspect_structure", "inspect_graph", "evaluate_impact", "evaluate_quality",
                    "assemble_context", "manage_indexes",
                ],
                "file_count": m.file_count,
                "symbol_count": m.symbol_count,
                "indexed_at": m.indexed_at,
                "languages": m.languages,
            });
            if let Some(w) = index_metadata_staleness_warning(&m.indexed_at) {
                if let Value::Object(ref mut map) = entry {
                    map.insert("staleness_warning".to_string(), json!(w));
                }
            }
            sources.push(entry);
        }

        for d in doc_values {
            let repo = d.get("source_id").and_then(|x| x.as_str()).unwrap_or("");
            let raw_doc_root = DocStore::open(self.config.doc_db_path(repo))
                .ok()
                .and_then(|s| s.root_path().ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let root_hint = path_map::remap(&raw_doc_root, &self.config.path_map, false);

            sources.push(json!({
                "source_id": repo,
                "display_name": repo,
                "root_hint": root_hint,
                "domains": ["docs"],
                "status": "indexed",
                "capabilities": [
                    "ingest_sources", "catalog_sources", "search_catalog", "read_content",
                    "inspect_structure", "assemble_context", "manage_indexes",
                ],
                "file_count": d.get("file_count"),
                "section_count": d.get("section_count"),
                "indexed_at": d.get("indexed_at"),
            }));
        }

        sources.sort_by(|a, b| {
            let sa = a.get("source_id").and_then(|x| x.as_str()).unwrap_or("");
            let sb = b.get("source_id").and_then(|x| x.as_str()).unwrap_or("");
            sa.cmp(sb)
        });

        self.success_response(&timer, json!({ "sources": sources }))
    }
}
