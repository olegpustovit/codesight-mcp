//! Dense-retrieval lane for `search_catalog`.
//!
//! Embeds the query once, runs per-store KNN, and returns a warning when
//! the model can't be loaded or no vec rows exist. Fusion with the lexical
//! lane happens in the caller via `retrieval::fusion::rrf`.

use serde_json::{Value, json};

use crate::service::CodeService;
use symslice_storage::storage::{IndexStore, doc_index::DocStore};

#[derive(Clone, Debug)]
pub(super) struct DenseRankedHit {
    pub(super) id: String,
    pub(super) distance: f64,
}

/// Result of asking the dense lane for ids. Warnings mirror §4.1 of the plan.
pub(super) struct DenseHits {
    pub(super) symbol_hits: Vec<DenseRankedHit>,
    pub(super) section_hits: Vec<DenseRankedHit>,
    pub(super) warning: Option<Value>,
}

/// Tier 5.2: surface dense-lane failures (model load, encode error,
/// missing vec table) as `NL_DEGRADED` so the caller can see they got
/// BM25-only results — not nothing — and the confidence drops to
/// `partial` automatically (NL mode is already partial regardless).
///
/// `reason` is one of: `model_load_failed`, `encoder_error`, `knn_error`,
/// `vectors_missing`, `dense_timeout`.
pub(super) fn semantic_fallback_warning(reason: &'static str, detail: String) -> Value {
    json!({
        "code": "NL_DEGRADED",
        "message": "Dense (semantic) retrieval unavailable; returning BM25-only results.",
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
                symbol_hits: Vec::new(),
                section_hits: Vec::new(),
                warning: Some(semantic_fallback_warning(
                    "model_load_failed",
                    reason.to_string(),
                )),
            };
        }
    };
    let qvec = match embedder.encode_query(query) {
        Ok(v) => v,
        Err(e) => {
            return DenseHits {
                symbol_hits: Vec::new(),
                section_hits: Vec::new(),
                warning: Some(semantic_fallback_warning("encoder_error", e.to_string())),
            };
        }
    };

    let mut symbol_hits = Vec::new();
    let mut section_hits = Vec::new();
    let mut missing: Vec<&'static str> = Vec::new();

    if let Some(store) = code_store {
        match store.query_symbol_knn(&qvec, k) {
            Ok(hits) => {
                symbol_hits = hits
                    .into_iter()
                    .map(|(id, distance)| DenseRankedHit { id, distance })
                    .collect()
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table") || msg.contains("vec_symbol_embeddings") {
                    missing.push("code");
                } else {
                    return DenseHits {
                        symbol_hits: Vec::new(),
                        section_hits: Vec::new(),
                        warning: Some(semantic_fallback_warning("knn_error", msg)),
                    };
                }
            }
        }
    }
    if let Some(store) = doc_store {
        match store.query_section_knn(&qvec, k) {
            Ok(hits) => {
                section_hits = hits
                    .into_iter()
                    .map(|(id, distance)| DenseRankedHit { id, distance })
                    .collect()
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("no such table") || msg.contains("vec_section_embeddings") {
                    missing.push("docs");
                } else {
                    return DenseHits {
                        symbol_hits: Vec::new(),
                        section_hits: Vec::new(),
                        warning: Some(semantic_fallback_warning("knn_error", msg)),
                    };
                }
            }
        }
    }

    let warning = if !missing.is_empty() && symbol_hits.is_empty() && section_hits.is_empty() {
        Some(semantic_fallback_warning(
            "vectors_missing",
            format!("no vec rows in: {}", missing.join(", ")),
        ))
    } else {
        None
    };

    DenseHits {
        symbol_hits,
        section_hits,
        warning,
    }
}
