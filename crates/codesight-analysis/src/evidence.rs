//! Reference-evidence analysis: collect import-edge, symbol-body, and
//! fallback content matches for a bare identifier.

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use codesight_language_support::imports_orchestration::{extract_imports, language_for_path};
use codesight_language_support::imports_types::ImportEdge;
use codesight_language_support::packages::extract_root_package_from_specifier;
use codesight_language_support::{FALLBACK_LANGUAGE, Language, builtin_registry};
use codesight_storage::storage::IndexStore;

use crate::graph as dep_graph;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Structural,
    Heuristic,
    Mixed,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceNote {
    pub signal: String,
    pub source: EvidenceSource,
    pub detail: String,
}

impl EvidenceNote {
    pub fn structural(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Structural,
            detail: detail.into(),
        }
    }

    pub fn heuristic(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Heuristic,
            detail: detail.into(),
        }
    }

    pub fn mixed(signal: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            signal: signal.into(),
            source: EvidenceSource::Mixed,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdentifierReferenceEvidence {
    pub identifier: String,
    pub import_files: Vec<String>,
    pub symbol_reference_files: Vec<String>,
    pub fallback_content_files: Vec<String>,
    pub evidence_notes: Vec<EvidenceNote>,
}

impl IdentifierReferenceEvidence {
    pub fn is_referenced(&self) -> bool {
        !self.import_files.is_empty()
            || !self.symbol_reference_files.is_empty()
            || !self.fallback_content_files.is_empty()
    }

    pub fn content_count(&self) -> usize {
        self.symbol_reference_files
            .iter()
            .chain(self.fallback_content_files.iter())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn referencing_files(&self) -> Vec<String> {
        self.import_files
            .iter()
            .chain(self.symbol_reference_files.iter())
            .chain(self.fallback_content_files.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone)]
struct FileImportFacts {
    file: String,
    language: String,
    edges: Vec<ImportEdge>,
}

pub struct ReferenceEvidenceProvider {
    file_imports: Vec<FileImportFacts>,
    symbol_sources: Vec<dep_graph::SymbolInfo>,
    raw_contents: Vec<(String, String)>,
}

impl ReferenceEvidenceProvider {
    pub fn from_store(store: &IndexStore) -> anyhow::Result<Self> {
        let raw_contents = store.all_file_contents().unwrap_or_default();
        let file_imports = raw_contents
            .iter()
            .map(|(file, content)| {
                let language = language_for_path(file).to_string();
                let edges = extract_imports(content, file, &language);
                FileImportFacts {
                    file: file.clone(),
                    language,
                    edges,
                }
            })
            .collect();

        Ok(Self {
            file_imports,
            symbol_sources: store.all_symbols_with_source()?,
            raw_contents,
        })
    }

    pub fn collect(&self, identifier: &str, search_content: bool) -> IdentifierReferenceEvidence {
        let mut import_files = BTreeSet::new();
        let mut symbol_reference_files = BTreeSet::new();
        let mut fallback_content_files = BTreeSet::new();
        let mut evidence_notes = Vec::new();

        for file_import in &self.file_imports {
            if file_import.edges.iter().any(|edge| {
                import_edge_mentions_identifier(edge, &file_import.language, identifier)
            }) {
                import_files.insert(file_import.file.clone());
            }
        }
        if !import_files.is_empty() {
            evidence_notes.push(EvidenceNote::structural(
                "import_edges",
                "import_count is derived from structural import edges, not raw import-line scanning",
            ));
        }

        for sym in &self.symbol_sources {
            if sym.name == identifier {
                continue;
            }
            if !sym
                .extraction_tier
                .fact_quality()
                .suitable_for_strict_analysis()
            {
                continue;
            }
            if codesight_language_support::text_match::word_match(&sym.source, identifier) {
                symbol_reference_files.insert(sym.file.clone());
            }
        }
        if !symbol_reference_files.is_empty() {
            evidence_notes.push(EvidenceNote::mixed(
                "symbol_source_references",
                "content_count includes identifier matches inside indexed symbol spans (trusted/baseline structural tiers only; heuristic-tier symbol bodies are not used here)",
            ));
        }

        if search_content {
            for (file, content) in &self.raw_contents {
                if import_files.contains(file) || symbol_reference_files.contains(file) {
                    continue;
                }
                let lang = language_for_file(file);
                if file_has_fallback_content_reference(content, identifier, lang) {
                    fallback_content_files.insert(file.clone());
                }
            }
            if !fallback_content_files.is_empty() {
                evidence_notes.push(EvidenceNote::heuristic(
                    "fallback_file_scan",
                    "content_count also includes whole-file fallback scans for top-level code outside indexed symbol spans",
                ));
            }
        }

        IdentifierReferenceEvidence {
            identifier: identifier.to_string(),
            import_files: import_files.into_iter().collect(),
            symbol_reference_files: symbol_reference_files.into_iter().collect(),
            fallback_content_files: fallback_content_files.into_iter().collect(),
            evidence_notes,
        }
    }
}

fn import_edge_mentions_identifier(edge: &ImportEdge, language: &str, identifier: &str) -> bool {
    if edge.names.iter().any(|name| name == identifier) {
        return true;
    }

    if edge.specifier == identifier {
        return true;
    }

    let root = extract_root_package_from_specifier(&edge.specifier, language);
    !root.is_empty() && root == identifier
}

fn language_for_file(file: &str) -> &'static dyn Language {
    builtin_registry()
        .extractor_for_path(Path::new(file))
        .unwrap_or(&FALLBACK_LANGUAGE)
}

fn file_has_fallback_content_reference(
    content: &str,
    identifier: &str,
    lang: &dyn Language,
) -> bool {
    let comment_prefixes = lang.line_comment_prefixes();
    let decl_prefixes = lang.declaration_line_prefixes();
    content.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }
        if comment_prefixes.iter().any(|p| trimmed.starts_with(p)) {
            return false;
        }
        if decl_prefixes.iter().any(|p| trimmed.starts_with(p)) {
            return false;
        }
        codesight_language_support::text_match::word_match(trimmed, identifier)
    })
}
