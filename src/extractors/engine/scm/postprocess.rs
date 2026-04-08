//! Postprocess phase for query-pack extraction.

use crate::model::StoredSymbol;

use super::{ScmExtractor, ScmMetadata};

#[derive(Clone, Copy)]
pub(super) enum PostprocessStep {
    CppHeaderLanguageFallback,
}

const DEFAULT_POSTPROCESS_STEPS: &[PostprocessStep] = &[PostprocessStep::CppHeaderLanguageFallback];

pub(super) fn postprocess_steps_for(_metadata: &ScmMetadata) -> &'static [PostprocessStep] {
    DEFAULT_POSTPROCESS_STEPS
}

pub(super) fn run_postprocesses(
    extractor: &ScmExtractor,
    file_path: &str,
    source: &str,
    symbols: &mut [StoredSymbol],
) {
    for step in extractor.compiled.postprocess_steps {
        match step {
            PostprocessStep::CppHeaderLanguageFallback => {
                crate::extractors::engine::cpp_header_language_fallback::adjust_language_for_cpp_header_if_plain_c(
                    symbols,
                    extractor.compiled.language_name,
                    file_path,
                    source,
                );
            }
        }
    }
}
