//! Python language implementation.

pub mod imports;

use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::PYTHON_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, PYTHON};
use crate::signature_density::measure_colon_after_name;
use crate::traits::{ImportExtractionMode, Language};

/// Python receiver detector — filters `self` / `cls` from parameter lists.
fn is_python_receiver(param: &str) -> bool {
    let name = param.split(':').next().unwrap_or(param).trim();
    matches!(name, "self" | "cls" | "*self" | "*cls")
}

pub struct Python;

fn load() -> TsLanguage {
    tree_sitter_python::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "python",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

fn python_root_package(specifier: &str) -> String {
    if specifier.starts_with('.') {
        return String::new();
    }
    specifier.split('.').next().unwrap_or("").to_string()
}

impl Language for Python {
    fn id(&self) -> LanguageId {
        PYTHON
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &PYTHON_MANIFEST
    }

    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }

    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }

    fn root_package_from_specifier(&self, specifier: &str) -> String {
        python_root_package(specifier)
    }

    fn imports_query(&self) -> Option<&'static str> {
        Some(include_str!("queries/imports.scm"))
    }

    fn branch_keywords(&self) -> &'static [&'static str] {
        &["if", "elif", "for", "while", "except", "and", "or"]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "__init__",
            "__new__",
            "__repr__",
            "__str__",
            "__eq__",
            "__ne__",
            "__lt__",
            "__le__",
            "__gt__",
            "__ge__",
            "__hash__",
            "__len__",
            "__iter__",
            "__next__",
            "__enter__",
            "__exit__",
            "__aenter__",
            "__aexit__",
            "__call__",
            "__getitem__",
            "__setitem__",
            "__delitem__",
            "__contains__",
            "__add__",
            "__sub__",
            "__mul__",
            "__div__",
            "__truediv__",
            "__mod__",
            "__bool__",
            "__format__",
            "__getattr__",
            "__setattr__",
            "setUp",
            "tearDown",
            "main",
        ]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &[]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["#"]
    }
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &["import ", "from ", "def ", "class ", "async def "]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        // Pytest + unittest convention: `test_foo` functions, `TestFoo`
        // classes.
        let name = id.leaf_name();
        name.starts_with("test_") || name.starts_with("Test")
    }

    fn normalise_body_for_dedup(&self, source: &str) -> Vec<u8> {
        // Python indentation is structurally significant — a body that nests
        // a block one level deeper is a different function, not the same
        // function with extra whitespace. Preserve the leading run by
        // measuring tab-as-4 indent depth and re-emitting the line with
        // spaces, while still stripping trailing whitespace and `#` line
        // comments.
        let mut out = String::with_capacity(source.len());
        for line in source.lines() {
            let trimmed = line.trim_end();
            if trimmed.trim_start().is_empty() {
                continue;
            }
            if trimmed.trim_start().starts_with('#') {
                continue;
            }
            let indent: usize = trimmed
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| if c == '\t' { 4 } else { 1 })
                .sum();
            if !out.is_empty() {
                out.push('\n');
            }
            for _ in 0..indent {
                out.push(' ');
            }
            out.push_str(trimmed.trim_start());
        }
        out.into_bytes()
    }

    fn resolve_import_edge(
        &self,
        edge: &ImportEdge,
        from_file: &str,
        source_files: &FxHashSet<String>,
        known_files_vec: &[String],
        alias_map: Option<&FxHashMap<String, Vec<String>>>,
        _workspace_crates: Option<&FxHashMap<String, String>>,
    ) -> Option<String> {
        if let Some(p) = crate::imports_orchestration::resolve_specifier(
            self,
            &edge.specifier,
            from_file,
            source_files,
            alias_map,
        ) {
            return Some(p);
        }
        if !edge.specifier.starts_with("./")
            && !edge.specifier.starts_with("../")
            && edge.specifier.contains('.')
        {
            let top = edge.specifier.split('.').next()?.to_string();
            if top.is_empty() {
                return None;
            }
            let resolved = imports::resolve_imports_to_files(&[top], known_files_vec, from_file);
            return resolved.into_iter().next();
        }
        None
    }

    fn top_level_import_stems(&self, source: &str, _path: &str) -> Vec<String> {
        let mut out = Vec::new();
        for edge in self.extract_imports(source) {
            let s = edge.specifier.trim();
            if s.is_empty() || s.starts_with('.') {
                continue;
            }
            if let Some(stem) = s.split('.').next().filter(|x| !x.is_empty()) {
                out.push(stem.to_string());
            }
        }
        out
    }

    fn display_import_lines_from_source(&self, content: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in content.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t.starts_with("import ") || t.starts_with("from ") {
                out.push(t.to_string());
            }
        }
        out
    }

    /// Python-idiomatic `import X` / `from X import a, b` rendering (ROADMAP
    /// P1.6). Called when edges are synthesised rather than read from source.
    fn display_import_lines(&self, edges: &[crate::imports_types::ImportEdge]) -> Vec<String> {
        edges
            .iter()
            .map(|e| {
                if e.names.is_empty() {
                    format!("import {}", e.specifier)
                } else {
                    format!("from {} import {}", e.specifier, e.names.join(", "))
                }
            })
            .collect()
    }

    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let pyproject = dir.join("pyproject.toml");
        if pyproject.is_file()
            && let Ok(content) = fs::read_to_string(&pyproject)
            && let Some(name) = parse_pyproject_name(&content)
        {
            return Some(name.replace('_', "-"));
        }
        let setup_cfg = dir.join("setup.cfg");
        if setup_cfg.is_file()
            && let Ok(content) = fs::read_to_string(&setup_cfg)
        {
            return parse_setup_cfg_name(&content);
        }
        None
    }

    fn pruned_dirs(&self) -> &'static [&'static str] {
        &["__pycache__", "venv", ".venv", ".tox"]
    }

    fn vendor_segments(&self) -> &'static [&'static str] {
        &[
            "__pycache__",
            "venv",
            ".venv",
            ".tox",
            "site-packages",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".eggs",
        ]
    }

    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
            "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
            "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
            "return", "try", "while", "with", "yield", "match", "case",
        ]
    }

    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &[
            "main.py",
            "__main__.py",
            "__init__.py",
            "app.py",
            "application.py",
            "wsgi.py",
            "asgi.py",
            "manage.py",
            "setup.py",
            "conftest.py",
        ]
    }

    fn test_file_basename_prefixes(&self) -> &'static [&'static str] {
        &["test_", "tests"]
    }

    fn test_file_suffixes(&self) -> &'static [&'static str] {
        &["_test.py"]
    }

    fn test_file_dirs(&self) -> &'static [&'static str] {
        &["/tests/", "/test/", "/__tests__/", "/spec/"]
    }

    fn is_conventional_path(&self, path: &str) -> bool {
        path.rsplit('/').next().unwrap_or(path) == "conftest.py"
    }

    fn conventional_dirs(&self) -> &'static [&'static str] {
        &["tests/", "scripts/", "fixtures/"]
    }

    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &["__init__.py"]
    }

    fn additional_live_roots(
        &self,
        root: Option<&Path>,
        all_files: &[String],
    ) -> Vec<crate::traits::LiveRoot> {
        use crate::traits::{LiveRoot, LiveRootConfidence};
        let Some(root) = root else {
            return Vec::new();
        };
        let mut roots = Vec::new();
        for file in all_files {
            if !file.ends_with(".py") {
                continue;
            }
            let abs = root.join(file);
            if let Ok(content) = fs::read_to_string(&abs)
                && content.contains("__name__")
                && content.contains("__main__")
            {
                roots.push(LiveRoot {
                    path: file.clone(),
                    reason: "main_guard",
                    message: format!("file '{file}' contains a __name__ == '__main__' style guard"),
                    confidence: LiveRootConfidence::Heuristic,
                    explicit: true,
                });
            }
        }
        roots
    }

    fn family(&self) -> LanguageFamily {
        LanguageFamily::ScriptedDynamic
    }
    fn nesting_mode(&self) -> NestingMode {
        NestingMode::Indent
    }
    fn fallback_type_baseline(&self) -> Option<f64> {
        Some(0.4)
    }
    fn supports_measured_types(&self) -> bool {
        true
    }
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_colon_after_name(source, self, false, is_python_receiver)
    }
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["any", "object"]
    }
    fn weak_type_substrings(&self) -> &'static [&'static str] {
        &["<any>", "typing.any"]
    }
}

fn parse_pyproject_name(content: &str) -> Option<String> {
    let table: toml::Value = content.parse().ok()?;
    table
        .get("project")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

fn parse_setup_cfg_name(content: &str) -> Option<String> {
    let mut in_metadata = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_metadata = trimmed == "[metadata]";
            continue;
        }
        if in_metadata && let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pyproject_name_wins_over_setup_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("pyproject.toml"),
            "[project]\nname = \"my_lib\"\n",
        )
        .unwrap();
        assert_eq!(
            Python.package_name_for_dir(tmp.path()),
            Some("my-lib".into())
        );
    }

    #[test]
    fn setup_cfg_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("setup.cfg"),
            "[metadata]\nname = other-lib\n",
        )
        .unwrap();
        assert_eq!(
            Python.package_name_for_dir(tmp.path()),
            Some("other-lib".into())
        );
    }
}
