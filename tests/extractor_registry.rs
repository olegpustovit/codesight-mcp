use std::collections::HashSet;
use std::path::Path;

use codesight_extractors::extractors::ExtractorRegistry;
use codesight_extractors::extractors::engine::catalogs::GrammarCatalog;
use codesight_core::ExtractionTier;

#[test]
fn supported_languages_and_curated_languages_are_distinct() {
    let registry = ExtractorRegistry::default();

    let supported: HashSet<&str> = registry.supported_languages().into_iter().collect();
    let curated: HashSet<&str> = registry.curated_languages().into_iter().collect();

    for language in ["ansible", "openapi"] {
        assert!(
            supported.contains(language),
            "{language} should be a supported language"
        );
        assert!(
            !curated.contains(language),
            "{language} should not be marked curated"
        );
    }

    for language in ["python", "json", "rust", "luau", "scss", "yaml", "sql"] {
        assert!(
            curated.contains(language),
            "{language} should be marked curated"
        );
    }
}

#[test]
fn built_in_grammars_have_structural_language_lookup() {
    let registry = ExtractorRegistry::default();
    let curated: HashSet<&str> = registry.curated_languages().into_iter().collect();

    for grammar in GrammarCatalog::builtin().iter() {
        let extractor = registry
            .extractor_for_language(grammar.language_name)
            .unwrap_or_else(|| panic!("missing language lookup for {}", grammar.language_name));
        let expected = if curated.contains(grammar.language_name) {
            ExtractionTier::CuratedQueryPack
        } else {
            ExtractionTier::GenericAstFallback
        };
        assert_eq!(
            extractor.extraction_tier(),
            expected,
            "language lookup for {} picked wrong tier",
            grammar.language_name
        );
    }
}

#[test]
fn query_backed_wrappers_expose_curated_tiers_for_path_and_language_lookup() {
    let registry = ExtractorRegistry::default();

    let yaml_path = registry
        .extractor_for_path(Path::new("settings.yaml"))
        .expect("yaml path extractor");
    assert_eq!(yaml_path.language_name(), "yaml");
    assert_eq!(
        yaml_path.extraction_tier(),
        ExtractionTier::CuratedQueryPack
    );

    let yaml_language = registry
        .extractor_for_language("yaml")
        .expect("yaml language extractor");
    assert_eq!(yaml_language.language_name(), "yaml");
    assert_eq!(
        yaml_language.extraction_tier(),
        ExtractionTier::CuratedQueryPack
    );

    let sql_path = registry
        .extractor_for_path(Path::new("schema.sql"))
        .expect("sql path extractor");
    assert_eq!(sql_path.language_name(), "sql");
    assert_eq!(sql_path.extraction_tier(), ExtractionTier::CuratedQueryPack);

    let sql_language = registry
        .extractor_for_language("sql")
        .expect("sql language extractor");
    assert_eq!(sql_language.language_name(), "sql");
    assert_eq!(
        sql_language.extraction_tier(),
        ExtractionTier::CuratedQueryPack
    );
}

#[test]
fn profile_paths_resolve_to_profiled_extractors_and_rewrite_symbols() {
    let registry = ExtractorRegistry::default();

    let openapi = registry
        .extractor_for_path(Path::new("api.openapi.json"))
        .expect("openapi profile extractor");
    assert_eq!(openapi.language_name(), "openapi");
    assert_eq!(openapi.extraction_tier(), ExtractionTier::Profiled);

    let openapi_symbols = openapi
        .extract(
            "api.openapi.json",
            r#"{"openapi":"3.0.0","info":{"title":"Test API"}}"#,
        )
        .expect("openapi profile extract");
    assert!(
        !openapi_symbols.is_empty(),
        "expected profiled openapi extraction to return symbols"
    );
    assert!(openapi_symbols.iter().all(|symbol| {
        symbol.language == "openapi" && symbol.extraction_tier == ExtractionTier::Profiled
    }));

    let ansible = registry
        .extractor_for_path(Path::new("playbooks/site.yml"))
        .expect("ansible profile extractor");
    assert_eq!(ansible.language_name(), "ansible");
    assert_eq!(ansible.extraction_tier(), ExtractionTier::Profiled);
}
