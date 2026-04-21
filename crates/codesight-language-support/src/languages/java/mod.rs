//! Java language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JAVA_MANIFEST;
use crate::manifest::{GrammarSpec, JAVA, LanguageId, LanguageManifest};
use crate::signature_density::measure_type_before_name;
use crate::traits::Language;

pub struct Java;

fn load() -> TsLanguage {
    tree_sitter_java::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "java",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

fn cfamily_is_restricted(source_text: &str, _name: &str) -> bool {
    let trimmed = source_text.trim_start();
    if trimmed.starts_with("private ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("internal ")
    {
        return true;
    }
    !trimmed.starts_with("public ")
}

impl Language for Java {
    fn id(&self) -> LanguageId {
        JAVA
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &JAVA_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn is_restricted(&self, src: &str, name: &str) -> bool {
        cfamily_is_restricted(src, name)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, src: &str) -> Option<f64> {
        measure_type_before_name(src, "java")
    }
}
