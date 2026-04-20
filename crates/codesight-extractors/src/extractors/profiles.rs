use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::ExtractionTier;

use super::LanguageExtractor;

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
    /// Resolve the host-profile attached to `path` via the language registry
    /// (LANGUAGES.md §3.3 step 6). Replaces the old `classify_special_path`
    /// duplicate with a route-through into
    /// `codesight-language-support::LanguageRegistry::classify_path`, which is
    /// the single source of truth for host/profile attachment.
    pub(crate) fn resolve_for_path(&self, path: &Path) -> Option<ResolvedProfile> {
        let registry = codesight_language_support::builtin_registry();
        let resolved = registry.classify_path(path);
        let profile = resolved.profile?;
        let host_name = registry.get(resolved.host)?.id().as_str();
        let (language_name, base_language) = match profile.as_str() {
            "openapi" => ("openapi", if host_name == "json" { "json" } else { "yaml" }),
            "ansible" => ("ansible", "yaml"),
            _ => return None,
        };
        Some(ResolvedProfile {
            language_name,
            base_language,
        })
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
            symbol.language = codesight_core::LanguageId::intern(self.language_name);
        }
        Ok(symbols)
    }
}
