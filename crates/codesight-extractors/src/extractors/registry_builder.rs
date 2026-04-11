//! Builds the default [`super::ExtractorRegistry`]: SCM query extractors plus overrides.

use std::path::Path;
use std::sync::Arc;

use tracing::warn;

use crate::adaptive_languages::project_extra_extensions;

use super::{
    AlExtractor, AsmExtractor, BladeExtractor, EjsExtractor, ExtractorRegistry, LessExtractor,
    LuauExtractor, RazorExtractor, SassExtractor, ScssExtractor, SqlExtractor, StylExtractor,
    VerseExtractor, VueExtractor, YamlExtractor, engine, profiles,
};

pub(crate) fn build_registry(project_root: Option<&Path>) -> ExtractorRegistry {
    let pack_catalog = match project_root {
        Some(root) => {
            engine::catalogs::QueryPackCatalog::for_workspace_root(root).unwrap_or_else(|error| {
                warn!(
                    root = %root.display(),
                    error = format!("{error:#}"),
                    "failed to load workspace query packs; using builtin catalog"
                );
                engine::catalogs::QueryPackCatalog::builtin()
            })
        }
        None => engine::catalogs::QueryPackCatalog::builtin(),
    };

    let mut extractors: Vec<Arc<dyn super::LanguageExtractor>> =
        engine::scm::build_scm_extractors_from_catalog(&pack_catalog)
            .expect("failed to build SCM extractors")
            .into_iter()
            .map(Arc::from)
            .collect();
    let curated_languages = extractors
        .iter()
        .map(|extractor| extractor.language_name().to_string())
        .collect::<std::collections::BTreeSet<_>>();

    if !curated_languages.contains("scss") {
        extractors.push(Arc::new(ScssExtractor));
    }
    extractors.push(Arc::new(SassExtractor));
    extractors.push(Arc::new(LessExtractor));
    extractors.push(Arc::new(StylExtractor));
    extractors.push(Arc::new(SqlExtractor));
    extractors.push(Arc::new(YamlExtractor));
    if !curated_languages.contains("luau") {
        extractors.push(Arc::new(LuauExtractor));
    }
    extractors.push(Arc::new(AsmExtractor));
    extractors.push(Arc::new(VueExtractor));
    extractors.push(Arc::new(AlExtractor));
    extractors.push(Arc::new(BladeExtractor));
    extractors.push(Arc::new(RazorExtractor));
    extractors.push(Arc::new(EjsExtractor));
    extractors.push(Arc::new(VerseExtractor));

    for grammar in engine::catalogs::GrammarCatalog::builtin().iter() {
        if curated_languages.contains(grammar.language_name) {
            continue;
        }
        extractors.push(Arc::new(
            engine::generic_ast::GenericAstExtractor::from_grammar(grammar),
        ));
    }

    let profiles = profiles::LanguageProfileRegistry;
    let profile_extractors = [
        ("ansible", "yaml"),
        ("openapi", "json"),
        ("openapi", "yaml"),
    ]
    .into_iter()
    .filter_map(|(profile_name, base_language)| {
        extractors
            .iter()
            .find(|extractor| extractor.language_name() == base_language)
            .cloned()
            .map(|delegate| {
                Arc::new(profiles::ProfiledExtractor::new(
                    profile_name,
                    base_language,
                    delegate,
                ))
            })
    })
    .collect();

    let mut registry =
        ExtractorRegistry::new_with_extractors(extractors, profile_extractors, profiles);
    if let Some(root) = project_root {
        let extra_extensions = project_extra_extensions(root);
        registry.apply_extra_extensions(&extra_extensions);
    }
    registry
}

pub(crate) fn build_default_registry() -> ExtractorRegistry {
    build_registry(None)
}
