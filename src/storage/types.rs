//! Public and internal types for the code index store.

use serde::Serialize;

use crate::core::symbols::SymbolKind;

/// Ranked symbol hit returned from code search APIs.
#[derive(Debug, Clone, Serialize)]
pub struct SearchSymbol {
    pub id: String,
    pub kind: SymbolKind,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub signature: String,
    pub summary: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzzy_similarity: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_distance: Option<u32>,
}

/// Symbol body retrieved for read_symbol-style tools.
#[derive(Debug, Clone, Serialize)]
pub struct RetrievedSymbol {
    pub id: String,
    pub kind: SymbolKind,
    pub file: String,
    pub line: u32,
    pub end_line: u32,
    pub source: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_drift: Option<bool>,
}

/// Lightweight symbol record for bulk analysis queries (dead code, hotspots, etc.).
#[derive(Debug, Clone)]
pub struct AnalysisSymbol {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub source_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SearchCandidate {
    pub(crate) result: SearchSymbol,
    pub(crate) qualified_name: String,
    pub(crate) raw_rank: f64,
}
