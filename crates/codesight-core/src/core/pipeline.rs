use super::language_id::LanguageId;
use super::symbols::{ExtractionTier, FactQuality, SymbolKind};

#[derive(Debug, Clone)]
pub struct StoredSymbol {
    pub id: String,
    pub extraction_tier: ExtractionTier,
    pub language: LanguageId,
    pub file_path: String,
    pub qualified_name: String,
    pub name: String,
    pub kind: SymbolKind,
    pub signature: String,
    pub summary: String,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: usize,
    pub end_byte: usize,
    pub source_hash: String,
    pub source_text: String,
    pub file_size_bytes: u64,
}

impl StoredSymbol {
    #[must_use]
    pub fn fact_quality(&self) -> FactQuality {
        self.extraction_tier.fact_quality()
    }
}

#[derive(Debug, Clone)]
pub struct FileFingerprint {
    pub file_path: String,
    pub language: LanguageId,
    pub size_bytes: u64,
    pub modified_ns: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct IndexedFileUpdate {
    pub file_path: String,
    pub language: LanguageId,
    pub size_bytes: u64,
    pub modified_ns: u64,
    pub content_hash: String,
    pub symbols: Vec<StoredSymbol>,
}
