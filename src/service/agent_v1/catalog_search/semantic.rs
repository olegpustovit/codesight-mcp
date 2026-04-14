//! Dense-retrieval lane for `search_catalog`.
//!
//! Embeds the query once, runs per-store KNN, and returns a warning when
//! the model can't be loaded or no vec rows exist. Fusion with the lexical
//! lane happens in the caller via `retrieval::fusion::rrf`.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::storage::{DocStore, IndexStore};

/// Result of asking the dense lane for ids. Warnings mirror §4.1 of the plan.
pub(super) struct DenseHits {
    pub(super) symbol_ids: Vec<String>,
    pub(super) section_ids: Vec<String>,
    pub(super) warning: Option<Value>,
}

fn fallback(reason: &'static str, detail: String) -> Value {
    json!({
        "code": "SEMANTIC_FALLBACK",
        "message": "Semantic retrieval unavailable; returning lexical results only.",
        "detail": { "reason": reason, "error": detail },
    })
}

/// Run the dense lane. Caller is expected to already have decided that
/// semantic retrieval was requested.
pub(super) fn run_dense_lane(
    service: &CodeService,
    query: &str,
    code_store: Option<&IndexStore>,
    doc_store: Option<&DocStore>,
    k: usize,
) -> DenseHits {
    let embedder = match service.embedder() {
        Ok(e) => e.clone(),
        Err(reason) => {
            return DenseHits {
                symbol_ids: Vec::new(),
                section_ids: Vec::new(),
                warning: Some(fallback("model_load_failed", reason.to_string())),
            };
        }
    };
    let qvec = match embedder.encode_query(query) {
        Ok(v) => v,
        Err(e) => {
            return DenseHits {
                symbol_ids: Vec::new(),
                section_ids: Vec::new(),
                warning: Some(fallback("encoder_error", e.to_string())),
            };
        }
    };

    let mut symbol_ids = Vec::new();
    let mut section_ids = Vec::new();
    let mut missing: Vec<&'static str> = Vec::new();

    if let Some(store) = code_store {
        match store.query_symbol_knn(&qvec, k) {
            Ok(hits) => symbol_ids = hits.into_iter().map(|(id, _)| id).collect(),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table") || msg.contains("vec_symbol_embeddings") {
                    missing.push("code");
                } else {
                    return DenseHits {
                        symbol_ids: Vec::new(),
                        section_ids: Vec::new(),
                        warning: Some(fallback("knn_error", msg)),
                    };
                }
            }
        }
    }
    if let Some(store) = doc_store {
        match store.query_section_knn(&qvec, k) {
            Ok(hits) => section_ids = hits.into_iter().map(|(id, _)| id).collect(),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table") || msg.contains("vec_section_embeddings") {
                    missing.push("docs");
                } else {
                    return DenseHits {
                        symbol_ids: Vec::new(),
                        section_ids: Vec::new(),
                        warning: Some(fallback("knn_error", msg)),
                    };
                }
            }
        }
    }

    let warning = if !missing.is_empty() && symbol_ids.is_empty() && section_ids.is_empty() {
        Some(fallback(
            "vectors_missing",
            format!("no vec rows in: {}", missing.join(", ")),
        ))
    } else {
        None
    };

    DenseHits {
        symbol_ids,
        section_ids,
        warning,
    }
}
