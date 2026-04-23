//! List indexed sources (`catalog_sources`).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::path_map;
use crate::service::CodeService;
use crate::service::agent_v1::args::CatalogSourcesArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::savings::MetaBuilder;
use codesight_storage::storage::doc_index::{DocStore, list_doc_summaries};
use codesight_storage::storage::list_repo_summaries;

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
        let _args: CatalogSourcesArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let timer = MetaBuilder::new(0);

        // Body cache: same generation ⇒ reuse the memoised payload. The
        // per-process `catalog_generation` counter is bumped on every
        // ingest success and on `manage_indexes` mutations, so any event
        // that could change this response invalidates automatically. The
        // freshness annotation (who's currently reindexing) is re-applied
        // downstream in dispatch, so the cached body stays correct even if
        // a reindex starts between a miss and the next hit.
        let observed_generation = self.catalog_generation();
        if let Some(mut body) = self.cached_catalog() {
            if let Value::Object(ref mut map) = body {
                let meta = map
                    .entry("_meta".to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Value::Object(meta_map) = meta {
                    meta_map.insert("cache_hit".to_string(), Value::Bool(true));
                }
            }
            return Ok(body);
        }

        // Route each code source through `ensure_repo_store` first so the
        // same auto-reindex-on-stale path that `manage_indexes status` and
        // `search_catalog` use is also applied here. Without this,
        // `catalog_sources` used to lag behind `manage_indexes status`
        // whenever the on-disk data had drifted since server start because
        // `list_repo_summaries` read raw metadata without triggering the
        // freshness check.
        //
        // Per-source `repo_summary()` + `root_path()` both go through
        // `SourceRegistry::snapshot`, so a 10-source catalog call does one
        // indexed SELECT per source after the initial warm-up instead of a
        // fresh DB open + 2 scans each time.
        let registry = self.source_registry();
        let initial_code_rows = list_repo_summaries(&self.config.repos_dir())?;
        let mut code_rows: Vec<_> = Vec::with_capacity(initial_code_rows.len());
        let mut root_paths: HashMap<String, String> = HashMap::with_capacity(initial_code_rows.len());
        for m in initial_code_rows {
            if let Ok(store) = self.ensure_repo_store(&m.repo, None)
                && let Ok(snap) = registry.snapshot(&m.repo, &store)
            {
                code_rows.push(snap.repo_summary.clone());
                root_paths.insert(
                    m.repo.clone(),
                    snap.root_path.to_string_lossy().to_string(),
                );
            } else {
                code_rows.push(m);
            }
        }
        let doc_values = list_doc_summaries(&self.config.docs_dir())?;

        let mut sources: Vec<Value> = Vec::new();

        for m in code_rows {
            let raw_root = root_paths.remove(&m.repo).unwrap_or_default();
            let root_hint = path_map::remap(&raw_root, &self.config.path_map, false);

            let mut entry = json!({
                "source_id": m.repo,
                "domain": "code",
                "stable_key": format!("{}#code", m.repo),
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
            if let Some(w) = index_metadata_staleness_warning(&m.indexed_at)
                && let Value::Object(ref mut map) = entry
            {
                map.insert("staleness_warning".to_string(), json!(w));
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
                "domain": "docs",
                "stable_key": format!("{repo}#docs"),
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

        let body = self.success_response(&timer, json!({ "sources": sources }))?;
        self.store_catalog(observed_generation, body.clone());
        Ok(body)
    }
}
