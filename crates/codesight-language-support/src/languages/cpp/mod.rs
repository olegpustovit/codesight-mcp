//! C++ language with `.h` header content probe that downgrades to plain C
//! when no C++-only construct is present.

use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::CPP_MANIFEST;
use crate::manifest::{C, CPP, GrammarSpec, LanguageId, LanguageManifest};
use crate::signature_density::{measure_type_before_name, normalize_type_token};
use crate::traits::Language;

pub struct Cpp;

fn load() -> TsLanguage {
    tree_sitter_cpp::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "cpp",
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

impl Language for Cpp {
    fn id(&self) -> LanguageId {
        CPP
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &CPP_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.cpp", "main.c"]
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
    fn postprocess_symbols(
        &self,
        symbols: &mut [codesight_core::StoredSymbol],
        file_path: &str,
        source: &str,
    ) {
        downgrade_plain_c_header_symbols(symbols, file_path, source);
    }
    fn type_token_score(&self, token: &str) -> f64 {
        let normalized = normalize_type_token(token);
        if normalized.is_empty() {
            return 0.0;
        }
        if matches!(normalized.as_str(), "void*" | "auto") {
            return 0.0;
        }
        1.0
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "switch", "case", "catch"]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "main",
            "operator=",
            "operator==",
            "operator!=",
            "operator<",
            "operator<=",
            "operator>",
            "operator>=",
            "operator+",
            "operator-",
            "operator*",
            "operator/",
            "operator[]",
            "operator()",
            "begin",
            "end",
            "cbegin",
            "cend",
            "size",
            "empty",
            "clear",
            "push_back",
            "pop_back",
            "front",
            "back",
            "insert",
            "erase",
            "find",
            "at",
            "swap",
        ]
    }
}

/// Rewrite emitted symbols' language from C++ to C when the file is a `.h`
/// header whose content has no C++-only constructs. Called from the `Cpp`
/// impl of `Language::postprocess_symbols` — generic code never branches
/// on "cpp" to reach this.
fn downgrade_plain_c_header_symbols(
    symbols: &mut [codesight_core::StoredSymbol],
    file_path: &str,
    source: &str,
) {
    if cpp_header_content_probe(Path::new(file_path), source).is_some() {
        for symbol in symbols.iter_mut() {
            symbol.language = C;
        }
    }
}

/// Returns `Some(C)` when a `.h` file parses as plain C.
pub fn cpp_header_content_probe(path: &Path, source: &str) -> Option<LanguageId> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    if !ext.eq_ignore_ascii_case("h") {
        return None;
    }
    if has_cpp_constructs(source) {
        return None;
    }
    Some(C)
}

fn has_cpp_constructs(source: &str) -> bool {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }
        if trimmed.starts_with("class ")
            || trimmed.starts_with("namespace ")
            || trimmed.starts_with("template")
            || trimmed.contains("template<")
            || trimmed.contains("template <")
            || trimmed.starts_with("using ")
            || trimmed.contains("::")
            || trimmed.contains("constexpr ")
            || trimmed.contains("noexcept")
            || trimmed.contains("nullptr")
            || trimmed.contains("virtual ")
            || trimmed.contains("override")
            || trimmed.starts_with("public:")
            || trimmed.starts_with("private:")
            || trimmed.starts_with("protected:")
            || trimmed.contains("[[nodiscard]]")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_c_header_resolves_to_c() {
        let src = "#ifndef FOO\n#define FOO\nvoid f(void);\n#endif\n";
        assert_eq!(cpp_header_content_probe(Path::new("foo.h"), src), Some(C));
    }

    #[test]
    fn cpp_header_with_namespace_stays_cpp() {
        let src = "namespace n { void f(); }\n";
        assert_eq!(cpp_header_content_probe(Path::new("foo.h"), src), None);
    }

    #[test]
    fn non_header_extension_not_probed() {
        assert_eq!(
            cpp_header_content_probe(Path::new("foo.cpp"), "void f();"),
            None
        );
    }
}
