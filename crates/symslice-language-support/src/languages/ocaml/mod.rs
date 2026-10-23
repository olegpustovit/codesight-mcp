//! OCaml implementation and interface structural navigation; no import resolution.
use crate::languages::builtins::{OCAML_INTERFACE_MANIFEST, OCAML_MANIFEST};
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, OCAML, OCAML_INTERFACE};
use crate::traits::Language;

pub struct OcamlSupport<const INTERFACE: bool>;

fn implementation() -> tree_sitter::Language {
    tree_sitter_ocaml::LANGUAGE_OCAML.into()
}
fn interface() -> tree_sitter::Language {
    tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "ocaml",
    loader: implementation,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};
pub static INTERFACE_GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "ocaml-interface",
    loader: interface,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};
impl<const INTERFACE: bool> Language for OcamlSupport<INTERFACE> {
    fn id(&self) -> LanguageId {
        if INTERFACE { OCAML_INTERFACE } else { OCAML }
    }
    fn manifest(&self) -> &'static LanguageManifest {
        if INTERFACE {
            &OCAML_INTERFACE_MANIFEST
        } else {
            &OCAML_MANIFEST
        }
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        self.manifest().grammar
    }
    fn is_identifier_node(&self, kind: &str) -> bool {
        matches!(
            kind,
            "value_name"
                | "module_name"
                | "module_type_name"
                | "type_constructor"
                | "class_name"
                | "class_type_name"
                | "method_name"
                | "field_name"
        )
    }
}

#[cfg(test)]
mod tests;
