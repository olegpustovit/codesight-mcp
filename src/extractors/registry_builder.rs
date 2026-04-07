//! Builds the default [`super::ExtractorRegistry`]: SCM query extractors plus overrides.

use super::{
    AlExtractor, AnsibleExtractor, AsmExtractor, BladeExtractor, EjsExtractor, ExtractorRegistry,
    LessExtractor, LuauExtractor, OpenApiExtractor, RazorExtractor, SassExtractor, ScssExtractor,
    SqlExtractor, StylExtractor, VerseExtractor, VueExtractor, YamlExtractor, engine,
};

pub(crate) fn build_default_registry() -> ExtractorRegistry {
    let mut extractors =
        engine::scm::build_scm_extractors().expect("failed to build SCM extractors");

    extractors.push(Box::new(ScssExtractor));
    extractors.push(Box::new(SassExtractor));
    extractors.push(Box::new(LessExtractor));
    extractors.push(Box::new(StylExtractor));
    extractors.push(Box::new(SqlExtractor));
    extractors.push(Box::new(YamlExtractor));
    extractors.push(Box::new(LuauExtractor));
    extractors.push(Box::new(AsmExtractor));
    extractors.push(Box::new(VueExtractor));
    extractors.push(Box::new(AlExtractor));
    extractors.push(Box::new(BladeExtractor));
    extractors.push(Box::new(RazorExtractor));
    extractors.push(Box::new(EjsExtractor));
    extractors.push(Box::new(VerseExtractor));

    ExtractorRegistry::new_with_extractors(
        extractors,
        Box::new(AnsibleExtractor),
        Box::new(OpenApiExtractor),
    )
}
