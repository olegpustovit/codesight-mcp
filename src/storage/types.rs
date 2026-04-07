//! Public and internal types for the code index store.

use crate::model::SearchSymbol;

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
