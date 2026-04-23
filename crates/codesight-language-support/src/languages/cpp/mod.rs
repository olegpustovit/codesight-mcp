//! C++ language with `.h` header content probe that downgrades to plain C
//! when no C++-only construct is present.

use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::languages::builtins::CPP_MANIFEST;
use crate::manifest::{C, CPP, GrammarSpec, LanguageId, LanguageManifest};
use crate::traits::{Language, TypeAnnotationStyle};
use crate::visibility::cfamily_is_restricted;

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
    fn postprocess_symbols(
        &self,
        symbols: &mut [codesight_core::StoredSymbol],
        file_path: &str,
        source: &str,
    ) {
        downgrade_plain_c_header_symbols(symbols, file_path, source);
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["void*", "auto"]
    }
    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "else if", "for", "while", "do", "switch", "case", "catch"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn distinctive_symbol_kinds(&self) -> &'static [&'static str] {
        &["class", "method", "namespace", "type"]
    }
    fn reclassify_file_language(
        &self,
        path: &str,
        has_distinctive_kinds: bool,
    ) -> Option<&'static str> {
        if !has_distinctive_kinds && path.ends_with(".h") {
            Some("c")
        } else {
            None
        }
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
