//! **CppHeaderLanguageFallback**: after SCM extraction with the `cpp` grammar, `.h` files that
//! appear to be plain C (no C++-only surface syntax) are reported as language `c`.
//!
//! Tree-sitter queries cannot reliably distinguish C headers from C++ headers when both parse
//! under the cpp grammar; this named heuristic encodes that policy in one place.

use std::path::Path;

use crate::model::StoredSymbol;

pub(crate) fn adjust_language_for_cpp_header_if_plain_c(
    symbols: &mut [StoredSymbol],
    scm_language_name: &str,
    file_path: &str,
    source: &str,
) {
    if scm_language_name != "cpp" {
        return;
    }
    let Some(ext) = Path::new(file_path).extension().and_then(|e| e.to_str()) else {
        return;
    };
    if !ext.eq_ignore_ascii_case("h") || has_cpp_constructs(source) {
        return;
    }
    for sym in symbols.iter_mut() {
        sym.language = "c".to_string();
    }
}

/// Returns true when source appears to use C++-only constructs (heuristic, line-based).
pub(crate) fn has_cpp_constructs(source: &str) -> bool {
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
    fn has_cpp_detects_namespace() {
        assert!(has_cpp_constructs("namespace n { int x; }"));
    }

    #[test]
    fn plain_c_header_no_cpp() {
        assert!(!has_cpp_constructs(
            "#ifndef FOO_H\n#define FOO_H\nvoid f(void);\n#endif\n"
        ));
    }

    #[test]
    fn adjust_relabels_symbols() {
        let mut syms = vec![StoredSymbol {
            id: "t::x#function".into(),
            language: "cpp".into(),
            file_path: "hdr.h".into(),
            qualified_name: "x".into(),
            name: "x".into(),
            kind: crate::model::SymbolKind::Function,
            signature: "void x()".into(),
            summary: "".into(),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 1,
            source_hash: "a".into(),
            source_text: "void x() {}".into(),
            file_size_bytes: 10,
        }];
        adjust_language_for_cpp_header_if_plain_c(&mut syms, "cpp", "hdr.h", "void x();");
        assert_eq!(syms[0].language, "c");
    }

    #[test]
    fn adjust_skips_when_cpp_construct_present() {
        let mut syms = vec![StoredSymbol {
            id: "t::x#function".into(),
            language: "cpp".into(),
            file_path: "hdr.h".into(),
            qualified_name: "x".into(),
            name: "x".into(),
            kind: crate::model::SymbolKind::Function,
            signature: "".into(),
            summary: "".into(),
            start_line: 1,
            end_line: 1,
            start_byte: 0,
            end_byte: 1,
            source_hash: "a".into(),
            source_text: "".into(),
            file_size_bytes: 0,
        }];
        adjust_language_for_cpp_header_if_plain_c(
            &mut syms,
            "cpp",
            "hdr.h",
            "namespace n { void x(); }",
        );
        assert_eq!(syms[0].language, "cpp");
    }
}
