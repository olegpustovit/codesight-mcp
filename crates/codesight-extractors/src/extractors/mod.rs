mod al;
mod asm;
mod blade;
pub(crate) mod classifier;
pub mod common;
pub(crate) mod containers;
mod ejs;
pub mod engine;
pub(crate) mod fallback_pipeline;
mod luau;
pub(crate) mod profiles;
mod razor;
mod registry_builder;
mod residual;
mod scss;
mod sql;
mod verse;
mod vue;
pub(crate) mod yaml;

use std::{collections::HashMap, path::Path, sync::Arc};

use tracing::warn;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::ExtractionTier;

pub use al::AlExtractor;
pub use asm::AsmExtractor;
pub use blade::BladeExtractor;
pub use ejs::EjsExtractor;
pub use luau::LuauExtractor;
pub use razor::RazorExtractor;
pub use scss::{LessExtractor, SassExtractor, ScssExtractor, StylExtractor};
pub use sql::SqlExtractor;
pub use verse::VerseExtractor;
pub use vue::VueExtractor;
pub use yaml::YamlExtractor;

/// Strategy pattern: each `LanguageExtractor` extracts symbols from source files.
///
/// Most languages are handled by `.scm` query files via [`engine::scm::ScmExtractor`].
/// Override extractors in this module handle languages without tree-sitter grammars
/// (assembly), container semantics, or domain-specific shaping.
pub trait LanguageExtractor: Send + Sync {
    fn language_name(&self) -> &'static str;
    fn supported_extensions(&self) -> &'static [&'static str];
    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::ManualOverride
    }
    fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>>;
}

pub struct ExtractorRegistry {
    extractors: Vec<Arc<dyn LanguageExtractor>>,
    profile_extractors: Vec<Arc<profiles::ProfiledExtractor>>,
    profiles: profiles::LanguageProfileRegistry,
    /// Best extractor per language for language-level capability lookup.
    /// This prefers structural tiers over richer path-specific overrides:
    /// curated query pack > generic AST fallback > manual override > profile wrapper.
    language_extractors: HashMap<String, Arc<dyn LanguageExtractor>>,
    /// Runtime extension overrides: maps lowercase extension (without dot) to
    /// the language name of an already-registered extractor.
    extra_extensions: HashMap<String, String>,
}

impl ExtractorRegistry {
    pub(crate) fn new_with_extractors(
        extractors: Vec<Arc<dyn LanguageExtractor>>,
        profile_extractors: Vec<Arc<profiles::ProfiledExtractor>>,
        profiles: profiles::LanguageProfileRegistry,
    ) -> Self {
        let language_extractors = build_language_lookup(&extractors, &profile_extractors);
        Self {
            extractors,
            profile_extractors,
            profiles,
            language_extractors,
            extra_extensions: HashMap::new(),
        }
    }
}

impl Default for ExtractorRegistry {
    fn default() -> Self {
        registry_builder::build_default_registry()
    }
}

impl ExtractorRegistry {
    pub fn for_workspace_root(root: &Path) -> Self {
        registry_builder::build_registry(Some(root))
    }

    /// Returns the set of languages this registry can serve by language name.
    pub fn supported_languages(&self) -> Vec<&str> {
        let mut langs: Vec<&str> = self
            .language_extractors
            .keys()
            .map(String::as_str)
            .collect();
        langs.sort();
        langs
    }

    /// Returns the set of all language names known to this registry.
    pub fn known_languages(&self) -> Vec<&str> {
        self.supported_languages()
    }

    pub fn curated_languages(&self) -> Vec<&str> {
        let mut langs: Vec<&str> = self
            .extractors
            .iter()
            .filter(|extractor| extractor.extraction_tier() == ExtractionTier::CuratedQueryPack)
            .map(|extractor| extractor.language_name())
            .collect();
        langs.sort();
        langs.dedup();
        langs
    }

    /// Lookup by language name prefers the best structural extractor for that language,
    /// even if path-based routing uses a richer manual override for some extensions.
    pub fn extractor_for_language(&self, language_name: &str) -> Option<&dyn LanguageExtractor> {
        let key = language_name.to_ascii_lowercase();
        self.language_extractors.get(&key).map(Arc::as_ref)
    }

    /// Apply extra extension-to-language mappings at runtime.
    pub fn apply_extra_extensions(&mut self, mappings: &HashMap<String, String>) -> Vec<String> {
        let mut warnings = Vec::new();
        let known: Vec<String> = self
            .supported_languages()
            .iter()
            .map(|s| s.to_string())
            .collect();

        for (ext_raw, lang_raw) in mappings {
            let ext = ext_raw.trim().to_string();
            let lang = lang_raw.trim().to_lowercase();

            if ext.is_empty() || ext == "." {
                let msg =
                    format!("extra_extensions: skipping empty extension for language '{lang_raw}'");
                warn!(message = %msg, "extra_extensions");
                warnings.push(msg);
                continue;
            }

            if lang.is_empty() {
                let msg =
                    format!("extra_extensions: skipping extension '{ext}' with empty language");
                warn!(message = %msg, "extra_extensions");
                warnings.push(msg);
                continue;
            }

            let ext_norm = ext.strip_prefix('.').unwrap_or(&ext).to_ascii_lowercase();

            if !known.iter().any(|k| k.eq_ignore_ascii_case(&lang)) {
                let msg = format!(
                    "extra_extensions: unknown language '{lang}' for extension '.{ext_norm}', skipping"
                );
                warn!(message = %msg, "extra_extensions");
                warnings.push(msg);
                continue;
            }

            self.extra_extensions.insert(ext_norm, lang);
        }
        warnings
    }

    pub fn extractor_for_path(&self, path: &Path) -> Option<&dyn LanguageExtractor> {
        if let Some(resolved) = self.profiles.resolve_for_path(path) {
            return self
                .profile_extractors
                .iter()
                .find(|extractor| extractor.matches(resolved))
                .map(|extractor| extractor.as_ref() as &dyn LanguageExtractor);
        }

        // --- Compound extension check (e.g. .blade.php before .php) ---
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            let lower = file_name.to_ascii_lowercase();
            // Check compound extensions first (before single-extension fallback)
            for extractor in &self.extractors {
                for ext in extractor.supported_extensions() {
                    if ext.contains('.')
                        && lower.ends_with(&format!(".{}", ext.to_ascii_lowercase()))
                    {
                        return Some(extractor.as_ref());
                    }
                }
            }
        }

        // --- Runtime extra_extensions override ---
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_ascii_lowercase();
            if let Some(lang) = self.extra_extensions.get(&ext_lower) {
                return self.find_extractor_by_language_name(lang);
            }
        }

        // --- Extension-based fallback ---
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        self.extractors
            .iter()
            .find(|extractor| {
                extractor.supported_extensions().iter().any(|candidate| {
                    !candidate.contains('.') && candidate.eq_ignore_ascii_case(extension.as_str())
                })
            })
            .map(Arc::as_ref)
    }

    fn find_extractor_by_language_name(
        &self,
        language_name: &str,
    ) -> Option<&dyn LanguageExtractor> {
        self.profile_extractors
            .iter()
            .find(|extractor| {
                extractor
                    .language_name()
                    .eq_ignore_ascii_case(language_name)
            })
            .map(|extractor| extractor.as_ref() as &dyn LanguageExtractor)
            .or_else(|| {
                self.extractors
                    .iter()
                    .find(|extractor| {
                        extractor
                            .language_name()
                            .eq_ignore_ascii_case(language_name)
                    })
                    .map(Arc::as_ref)
            })
    }
}

fn build_language_lookup(
    extractors: &[Arc<dyn LanguageExtractor>],
    profile_extractors: &[Arc<profiles::ProfiledExtractor>],
) -> HashMap<String, Arc<dyn LanguageExtractor>> {
    let mut lookup = HashMap::new();
    for extractor in extractors {
        insert_best_language_extractor(&mut lookup, extractor.clone());
    }
    for extractor in profile_extractors {
        let erased: Arc<dyn LanguageExtractor> = extractor.clone();
        insert_best_language_extractor(&mut lookup, erased);
    }
    lookup
}

fn insert_best_language_extractor(
    lookup: &mut HashMap<String, Arc<dyn LanguageExtractor>>,
    candidate: Arc<dyn LanguageExtractor>,
) {
    let key = candidate.language_name().to_ascii_lowercase();
    let should_replace = lookup.get(&key).is_none_or(|current| {
        language_lookup_priority(candidate.extraction_tier())
            < language_lookup_priority(current.extraction_tier())
    });
    if should_replace {
        lookup.insert(key, candidate);
    }
}

fn language_lookup_priority(tier: ExtractionTier) -> u8 {
    match tier {
        ExtractionTier::CuratedQueryPack => 0,
        ExtractionTier::GenericAstFallback => 1,
        ExtractionTier::ManualOverride => 2,
        ExtractionTier::Profiled => 3,
    }
}
