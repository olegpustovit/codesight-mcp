//! Semantic search integration for `search_catalog`.
//!
//! Loads a per-source TF-IDF embedding store, embeds the query, and either
//! replaces (`semantic_only`) or hybrid-merges (`sem_weight > 0`) the results
//! into the item list produced by the BM25 phase.

use std::collections::HashSet;

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::SearchCatalogArgs;

pub(super) enum SemanticMode {
    Bm25,
    Hybrid,
    Only,
}

impl SemanticMode {
    pub(super) fn resolve(enabled: bool, only: bool) -> Self {
        if only {
            Self::Only
        } else if enabled {
            Self::Hybrid
        } else {
            Self::Bm25
        }
    }

    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Bm25 => "bm25",
            Self::Hybrid => "hybrid",
            Self::Only => "semantic_only",
        }
    }
}

pub(super) fn apply_semantic(
    service: &CodeService,
    args: &SearchCatalogArgs,
    limit: usize,
    mode: &SemanticMode,
    items: &mut Vec<Value>,
) {
    let emb_path = service.config.embedding_path(&args.source_id);
    let model_path = emb_path.with_extension("tfidf.json");
    if !emb_path.exists() || !model_path.exists() {
        return;
    }
    let emb_store = crate::retrieval::semantic::EmbeddingStore::open(&emb_path);
    let Ok(model) = crate::retrieval::semantic::TfidfModel::load(&model_path) else {
        return;
    };
    let query_vec = crate::retrieval::semantic::compute_query_vector(
        &args.query,
        &model.vocab,
        model.doc_count,
        &model.df,
    );
    let sem_results = emb_store.search(&query_vec, limit);
    let sem_weight = args.semantic_weight.unwrap_or(0.5);

    let Ok(store) = service.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) else {
        return;
    };

    match mode {
        SemanticMode::Only => {
            items.clear();
            for (sym_id, score) in &sem_results {
                if let Ok(Some(rec)) = store.symbol_record(sym_id) {
                    items.push(build_semantic_item(args, &rec, *score));
                }
            }
        }
        SemanticMode::Hybrid if sem_weight > 0.0 => {
            let existing_ids: HashSet<String> = items
                .iter()
                .filter_map(|i| i["ref"]["id"].as_str().map(|s| s.to_string()))
                .collect();
            for (sym_id, score) in &sem_results {
                if existing_ids.contains(sym_id) {
                    continue;
                }
                if let Ok(Some(rec)) = store.symbol_record(sym_id) {
                    items.push(build_semantic_item(args, &rec, score * sem_weight));
                }
            }
            items.sort_by(|a, b| {
                let sa = a["score"].as_f64().unwrap_or(0.0);
                let sb = b["score"].as_f64().unwrap_or(0.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        _ => {}
    }
}

fn build_semantic_item(
    args: &SearchCatalogArgs,
    rec: &crate::storage::SymbolRecord,
    score: f64,
) -> Value {
    let title = rec
        .id
        .rsplit("::")
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("");
    let summary = rec.stored_source.lines().next().unwrap_or("");
    json!({
        "ref": {
            "id": rec.id,
            "stable_id": rec.id,
            "source_id": args.source_id,
            "domain": "code",
            "kind": "symbol",
            "path": rec.file_path,
            "title": title,
            "summary": summary,
        },
        "snippet": summary,
        "score": crate::service::agent_v1::util::round_score(score),
        "line": rec.start_line,
    })
}
