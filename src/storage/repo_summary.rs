use std::collections::BTreeMap;

use serde::Serialize;

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
