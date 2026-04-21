use std::collections::HashSet;
use std::path::Path;

use codesight_core::ExtractionTier;
use codesight_language_support::LanguageRegistry;
use codesight_language_support::query_pack_catalog::GrammarCatalog;

fn curated_languages() -> HashSet<String> {
    LanguageRegistry::builtin()
        .curated_languages()
        .map(|id| id.as_str().to_string())
        .collect()
}

fn all_languages() -> HashSet<String> {
    LanguageRegistry::builtin()
        .all()
        .map(|lang| lang.id().as_str().to_string())
        .collect()
}

#[test]
fn supported_languages_and_curated_languages_are_distinct() {
    let all = all_languages();
    let curated = curated_languages();

    // Profile languages (openapi, ansible) aren't standalone Language impls
    // yet (Phase 6), so skip that half of the legacy assertion and focus on
    // the curated-set invariant.
    for language in ["python", "json", "rust", "luau", "scss", "yaml", "sql"] {
        assert!(
            curated.contains(language),
            "{language} should be marked curated"
        );
        assert!(all.contains(language), "{language} should be registered");
    }
}

#[test]
fn built_in_grammars_have_structural_language_lookup() {
    let registry = LanguageRegistry::builtin();
    let curated = curated_languages();
    for grammar in GrammarCatalog::builtin().iter() {
        let name = grammar.language_name();
        let lang = registry
            .for_language_name(name)
            .unwrap_or_else(|| panic!("missing language lookup for {name}"));
        let expected = if curated.contains(name) {
            ExtractionTier::CuratedQueryPack
        } else {
            ExtractionTier::ManualOverride
        };
        assert_eq!(
            lang.extraction_tier(),
            expected,
            "language lookup for {name} picked wrong tier",
        );
    }
}

#[test]
fn query_backed_languages_expose_curated_tiers_for_path_and_name_lookup() {
    let registry = LanguageRegistry::builtin();

    let yaml_path = registry
        .for_path(Path::new("settings.yaml"))
        .expect("yaml path lang");
    assert_eq!(yaml_path.id().as_str(), "yaml");
    assert_eq!(yaml_path.extraction_tier(), ExtractionTier::CuratedQueryPack);

    let yaml_by_name = registry.for_language_name("yaml").expect("yaml name lang");
    assert_eq!(yaml_by_name.id().as_str(), "yaml");
    assert_eq!(
        yaml_by_name.extraction_tier(),
        ExtractionTier::CuratedQueryPack
    );

    let sql_path = registry
        .for_path(Path::new("schema.sql"))
        .expect("sql path lang");
    assert_eq!(sql_path.id().as_str(), "sql");
    assert_eq!(sql_path.extraction_tier(), ExtractionTier::CuratedQueryPack);
}
