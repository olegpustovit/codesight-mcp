//! XML language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::XML_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, XML};
use crate::traits::Language;

pub struct XmlSupport;

fn load() -> TsLanguage {
    tree_sitter_xml::LANGUAGE_XML.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "xml",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for XmlSupport {
    fn id(&self) -> LanguageId {
        XML
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &XML_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::DeclarativeMarkup
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn is_reference_target(&self) -> bool {
        false
    }
}
