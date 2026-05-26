//! C# language.

use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::CSHARP_MANIFEST;
use crate::manifest::{CSHARP, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::{Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

pub struct CSharp;

fn load() -> TsLanguage {
    tree_sitter_c_sharp::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "c_sharp",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

impl Language for CSharp {
    fn id(&self) -> LanguageId {
        CSHARP
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &CSHARP_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }

    /// C#-idiomatic `using Namespace;` (ROADMAP P1.6).
    fn display_import_lines(&self, edges: &[crate::imports_types::ImportEdge]) -> Vec<String> {
        let mut lines = Vec::new();
        for e in edges {
            if e.names.is_empty() {
                lines.push(format!("using {};", e.specifier));
            } else {
                for n in &e.names {
                    lines.push(format!("using {}.{};", e.specifier, n));
                }
            }
        }
        lines
    }

    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("csproj") {
                continue;
            }
            let Ok(content) = fs::read_to_string(&path) else {
                continue;
            };
            if let Some(name) = extract_csproj_tag(&content, "PackageName") {
                return Some(name);
            }
            if let Some(name) = extract_csproj_tag(&content, "AssemblyName") {
                return Some(name);
            }
        }
        None
    }

    fn pruned_dirs(&self) -> &'static [&'static str] {
        &["bin", "obj"]
    }

    fn vendor_segments(&self) -> &'static [&'static str] {
        &["bin", "obj"]
    }

    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "abstract",
            "as",
            "base",
            "bool",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "checked",
            "class",
            "const",
            "continue",
            "decimal",
            "default",
            "delegate",
            "do",
            "double",
            "else",
            "enum",
            "event",
            "explicit",
            "extern",
            "false",
            "finally",
            "fixed",
            "float",
            "for",
            "foreach",
            "goto",
            "if",
            "implicit",
            "in",
            "int",
            "interface",
            "internal",
            "is",
            "lock",
            "long",
            "namespace",
            "new",
            "null",
            "object",
            "operator",
            "out",
            "override",
            "params",
            "private",
            "protected",
            "public",
            "readonly",
            "ref",
            "return",
            "sbyte",
            "sealed",
            "short",
            "sizeof",
            "stackalloc",
            "static",
            "string",
            "struct",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "uint",
            "ulong",
            "unchecked",
            "unsafe",
            "ushort",
            "using",
            "virtual",
            "void",
            "volatile",
            "while",
            "async",
            "await",
            "yield",
            "var",
            "dynamic",
            "partial",
            "nameof",
            "record",
            "init",
            "nint",
            "nuint",
        ]
    }

    fn is_restricted(&self, src: &str, _name: &str) -> bool {
        cfamily_is_restricted(src)
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
        &["dynamic", "object"]
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &[
            "if", "else if", "for", "foreach", "while", "do", "switch", "case", "catch",
        ]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "Equals",
            "GetHashCode",
            "ToString",
            "CompareTo",
            "Dispose",
            "Finalize",
            "Clone",
            "GetEnumerator",
            "MoveNext",
            "Reset",
            "Current",
            "Main",
            "Configure",
            "ConfigureServices",
            "OnGet",
            "OnPost",
            "OnPut",
            "OnDelete",
            "Invoke",
            "InvokeAsync",
            "ExecuteAsync",
            "HandleAsync",
        ]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        // MSTest / xUnit / NUnit conventions: method names starting with
        // `Test` / `Should` / `When` are common, but file-level test
        // detection is the more reliable signal. Keep this narrow.
        id.leaf_name().starts_with("Test")
    }
}

fn extract_csproj_tag(content: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)?;
    let after_open = start + open.len();
    let end = content[after_open..].find(&close)?;
    let value = content[after_open..after_open + end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
