use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct FileContentArgs {
    pub source_id: String,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub end_line: Option<u32>,
    #[serde(default)]
    pub repo_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetSectionArgs {
    pub source_id: String,
    #[serde(default)]
    pub section_id: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub verify: bool,
    #[serde(default)]
    pub repo_path: Option<String>,
}

pub(crate) fn default_max_results() -> usize {
    10
}

pub struct IndexResult {
    pub repo: String,
    pub indexed_at: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub languages: BTreeMap<String, usize>,
    pub languages_without_import_extractor: Vec<String>,
}

pub struct DocIndexResult {
    pub repo: String,
    pub indexed_at: String,
    pub file_count: usize,
    pub section_count: usize,
}
