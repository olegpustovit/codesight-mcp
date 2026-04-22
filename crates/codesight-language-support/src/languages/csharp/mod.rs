//! C# language.

use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::CSHARP_MANIFEST;
use crate::manifest::{CSHARP, GrammarSpec, LanguageId, LanguageManifest};
use crate::signature_density::{measure_type_before_name, normalize_type_token};
use crate::traits::Language;

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
        measure_type_before_name(src, self)
    }
    fn type_token_score(&self, token: &str) -> f64 {
        let normalized = normalize_type_token(token);
        if normalized.is_empty() {
            return 0.0;
        }
        if matches!(normalized.as_str(), "dynamic" | "object") {
            return 0.0;
        }
        1.0
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
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
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
    if value.is_empty() { None } else { Some(value.to_string()) }
}
