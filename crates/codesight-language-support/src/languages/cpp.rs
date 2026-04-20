//! C / C++ identity. `.h` is a shared header extension; we claim it for
//! C++ on the path-only pass and use a content probe to downgrade to plain
//! C when no C++-only construct is present (LANGUAGES.md §3.3 step 5,
//! §5 `cpp_header_language_fallback.rs` migration row).

use std::path::Path;

use crate::manifest::{C, CPP, LanguageId};

/// Returns `Some(C)` when a `.h` file parses as plain C (no C++-only
/// constructs). Returns `None` (leaves host classification alone) for any
/// other path or when C++ constructs are present.
///
/// Mirrors `codesight-extractors::cpp_header_language_fallback::
/// has_cpp_constructs`. Phase B keeps the heuristic here so downstream
/// identity sees `c`; Phase D removes the legacy duplicate once every call
/// site reads identity from the registry.
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

/// Marker kept exported so `CPP` can be referenced unambiguously from
/// manifest tables (and so `rustc` does not see the `use` as unused).
pub const CPP_LANGUAGE: LanguageId = CPP;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    #[test]
    fn comment_lines_not_treated_as_cpp_constructs() {
        let src = "// contains class keyword but in comment\nvoid f(void);\n";
        assert_eq!(cpp_header_content_probe(Path::new("foo.h"), src), Some(C));
    }
}
