//! OpenAPI — host-profile variant of YAML and JSON, matched by the
//! `openapi` / `swagger` stems on `.json` / `.yaml` / `.yml` files.
//! Grammar is chosen based on the file extension at `for_path` time.

use std::path::Path;

use dowser_core::StoredSymbol;
use dowser_core::symbols::ExtractionTier;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::OPENAPI_MANIFEST;
use crate::languages::yaml;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, OPENAPI};
use crate::traits::Language;

pub struct Openapi;

fn is_json_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

impl Language for Openapi {
    fn id(&self) -> LanguageId {
        OPENAPI
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &OPENAPI_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        // Default to YAML grammar — callers that care about the JSON case
        // (content extraction) re-enter extract_symbols which dispatches.
        Some(&yaml::GRAMMAR)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::ManualOverride
    }
    fn extract_symbols(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        if is_json_path(Path::new(file_path)) {
            crate::languages::json::JsonSupport.extract_symbols(file_path, source)
        } else {
            crate::languages::yaml::extractor::YamlExtractor.extract(file_path, source)
        }
    }
}
