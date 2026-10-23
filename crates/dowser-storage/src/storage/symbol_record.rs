use std::path::PathBuf;

use serde::Serialize;

use crate::core::symbols::{ExtractionTier, FactQuality, SymbolKind};

#[derive(Debug, Clone, Serialize)]
pub struct SymbolMetadata {
    pub id: String,
    pub file: String,
    pub file_size_bytes: u64,
    pub symbol_size_bytes: u64,
    pub estimated_file_tokens: u64,
    pub estimated_symbol_tokens: u64,
    pub estimated_savings: u64,
}

#[derive(Debug, Clone)]
pub struct SymbolRecord {
    pub id: String,
    pub kind: SymbolKind,
    pub extraction_tier: ExtractionTier,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub source_hash: String,
    pub stored_source: String,
    pub file_size_bytes: u64,
    pub root_path: PathBuf,
}

impl SymbolRecord {
    #[must_use]
    pub fn fact_quality(&self) -> FactQuality {
        self.extraction_tier.fact_quality()
    }
}
