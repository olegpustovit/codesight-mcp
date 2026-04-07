use std::{collections::BTreeMap, collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use strum::{EnumString, IntoStaticStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Constant,
    Type,
    Template,
    Import,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    pub fn from_db(value: &str) -> Option<Self> {
        value.parse().ok()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    pub tokens_saved: u64,
    pub total_tokens_saved: u64,
    pub response_tokens: u64,
    pub naive_tokens: u64,
    pub cost_avoided: HashMap<String, f64>,
    pub total_cost_avoided: HashMap<String, f64>,
    pub retrieval_engine: String,
    pub retrieval_version: String,
    pub timing_ms: f64,
    pub powered_by: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoMetadata {
    pub repo: String,
    pub indexed_at: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub languages: BTreeMap<String, usize>,
    pub index_version: u32,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub estimated_naive_tokens: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub index_size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_stale: Option<bool>,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

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

#[derive(Debug, Clone)]
pub struct StoredSymbol {
    pub id: String,
    pub language: String,
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

#[derive(Debug, Clone)]
pub struct FileFingerprint {
    pub file_path: String,
    pub language: String,
    pub size_bytes: u64,
    pub modified_ns: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct IndexedFileUpdate {
    pub file_path: String,
    pub language: String,
    pub size_bytes: u64,
    pub modified_ns: u64,
    pub content_hash: String,
    pub symbols: Vec<StoredSymbol>,
}

#[derive(Debug, Clone)]
pub struct SymbolRecord {
    pub id: String,
    pub kind: SymbolKind,
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
