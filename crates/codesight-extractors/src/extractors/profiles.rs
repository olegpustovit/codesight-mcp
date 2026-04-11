use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::ExtractionTier;

use super::LanguageExtractor;
use super::classifier::{SpecialExtractor, classify_special_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedProfile {
    language_name: &'static str,
    base_language: &'static str,
}

impl ResolvedProfile {
    pub(crate) fn language_name(self) -> &'static str {
        self.language_name
    }

    pub(crate) fn base_language(self) -> &'static str {
        self.base_language
    }
}

#[derive(Clone, Default)]
pub(crate) struct LanguageProfileRegistry;

impl LanguageProfileRegistry {
    pub(crate) fn resolve_for_path(&self, path: &Path) -> Option<ResolvedProfile> {
        match classify_special_path(path)? {
            SpecialExtractor::Ansible => Some(ResolvedProfile {
                language_name: "ansible",
                base_language: "yaml",
            }),
            SpecialExtractor::OpenApi => {
                let base_language = match path.extension().and_then(|ext| ext.to_str()) {
                    Some(ext) if ext.eq_ignore_ascii_case("json") => "json",
                    _ => "yaml",
                };
                Some(ResolvedProfile {
                    language_name: "openapi",
                    base_language,
                })
            }
        }
    }
}

pub(crate) struct ProfiledExtractor {
    language_name: &'static str,
    base_language: &'static str,
    delegate: Arc<dyn LanguageExtractor>,
}

impl ProfiledExtractor {
    pub(crate) fn new(
        language_name: &'static str,
        base_language: &'static str,
        delegate: Arc<dyn LanguageExtractor>,
    ) -> Self {
        Self {
            language_name,
            base_language,
            delegate,
        }
    }

    pub(crate) fn matches(&self, resolved: ResolvedProfile) -> bool {
        self.language_name == resolved.language_name()
            && self.base_language == resolved.base_language()
    }
}

impl LanguageExtractor for ProfiledExtractor {
    fn language_name(&self) -> &'static str {
        self.language_name
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::Profiled
    }

    fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let mut symbols = self.delegate.extract(file_path, source)?;
        for symbol in &mut symbols {
            symbol.extraction_tier = ExtractionTier::Profiled;
            symbol.language = self.language_name.to_string();
        }
        Ok(symbols)
    }
}
