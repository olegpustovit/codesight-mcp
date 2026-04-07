//! **OpenAPI / Swagger** specs can share `.json` / `.yaml` with non-spec files; basename-based
//! routing ([`super::classifier`]) plus this delegate keeps `language_name == "openapi"` consistent.
use crate::model::StoredSymbol;

use super::LanguageExtractor;
use super::yaml::YamlExtractor;

/// Delegates to the SCM JSON extractor or [`YamlExtractor`] by extension; reports `"openapi"`.
pub struct OpenApiExtractor;

impl LanguageExtractor for OpenApiExtractor {
    fn language_name(&self) -> &'static str {
        "openapi"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &[]
    }

    fn extract(&self, file_path: &str, source: &str) -> anyhow::Result<Vec<StoredSymbol>> {
        let lower = file_path.to_ascii_lowercase();
        if lower.ends_with(".json") {
            // Use the SCM JSON extractor
            let json_source = super::engine::scm::QUERY_FILES
                .iter()
                .find(|(k, _)| *k == "json")
                .map(|(_, s)| *s)
                .expect("json query file not found");
            let meta = super::engine::scm::parse_scm_header(json_source);
            let ext = super::engine::scm::ScmExtractor::new(&meta, json_source)?;
            ext.extract(file_path, source)
        } else {
            YamlExtractor.extract(file_path, source)
        }
    }
}
