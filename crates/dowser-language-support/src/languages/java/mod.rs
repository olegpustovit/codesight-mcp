//! Java language.

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::JAVA_MANIFEST;
use crate::manifest::{GrammarSpec, JAVA, LanguageId, LanguageManifest};
use crate::traits::{Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

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
    fn is_restricted(&self, src: &str, _name: &str) -> bool {
        cfamily_is_restricted(src)
    }
    /// Java-idiomatic `import pkg.Class;` (ROADMAP P1.6). Overrides the
    /// default ES-module shape so Java contexts show real Java imports.
    fn display_import_lines(&self, edges: &[crate::imports_types::ImportEdge]) -> Vec<String> {
        let mut lines = Vec::new();
        for e in edges {
            if e.names.is_empty() {
                lines.push(format!("import {};", e.specifier));
            } else {
                for n in &e.names {
                    lines.push(format!("import {}.{};", e.specifier, n));
                }
            }
        }
        lines
    }
    fn family(&self) -> LanguageFamily {
        LanguageFamily::BracedStatic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Brace
    }
    fn type_annotation_style(&self) -> TypeAnnotationStyle {
        TypeAnnotationStyle::TypeBeforeName
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["object"]
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &[
            "if", "else if", "for", "while", "do", "switch", "case", "catch",
        ]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn regex_reachability_patterns(&self) -> &'static [(dowser_core::EdgeKind, &'static str)] {
        // Java: `import a.b.C;` and `import static a.b.C.foo;`. Capture
        // the trailing identifier as a symbol edge.
        &[(
            dowser_core::EdgeKind::ImportStatement,
            r#"^\s*import\s+(?:static\s+)?[\w\.]+\.(\w+)\s*;"#,
        )]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "equals",
            "hashCode",
            "toString",
            "compareTo",
            "clone",
            "finalize",
            "iterator",
            "hasNext",
            "next",
            "remove",
            "size",
            "isEmpty",
            "get",
            "set",
            "put",
            "add",
            "contains",
            "run",
            "call",
            "close",
            "main",
            "builder",
            "build",
            "of",
            "valueOf",
            "getInstance",
            "newInstance",
        ]
    }
    fn is_test_symbol_id(&self, id: dowser_core::SymbolIdRef<'_>) -> bool {
        // JUnit 4/5 convention: `test…` methods plus `@Test`-annotated
        // ones. We only see leaf names, so match the prefix.
        id.leaf_name().starts_with("test")
    }
    fn vendor_segments(&self) -> &'static [&'static str] {
        &[".gradle", ".idea"]
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "abstract",
            "assert",
            "boolean",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extends",
            "final",
            "finally",
            "float",
            "for",
            "goto",
            "if",
            "implements",
            "import",
            "instanceof",
            "int",
            "interface",
            "long",
            "native",
            "new",
            "package",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "static",
            "strictfp",
            "super",
            "switch",
            "synchronized",
            "this",
            "throw",
            "throws",
            "transient",
            "try",
            "void",
            "volatile",
            "while",
            "true",
            "false",
            "null",
            "var",
            "yield",
            "record",
            "sealed",
            "non-sealed",
            "permits",
        ]
    }
}
