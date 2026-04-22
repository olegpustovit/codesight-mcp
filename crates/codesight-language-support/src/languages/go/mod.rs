//! Go language implementation.

use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::GO_MANIFEST;
use crate::manifest::{GO, GrammarSpec, LanguageId, LanguageManifest};
use crate::signature_density::{
    find_matching_delimiter, normalize_type_token, split_top_level, take_until_top_level_char,
};
use crate::traits::{ImportExtractionMode, Language};

/// Go-specific signature density: `func (recv) name(params) ret {...}`.
/// Handles grouped parameters (`x, y int`), variadic (`...T`), and the
/// Go convention where a missing return type counts as typed (implicit void).
fn measure_go_signature(lang: &Go, source: &str) -> Option<f64> {
    let func_idx = source.find("func")?;
    let mut scan = source[func_idx + 4..].trim_start();
    if scan.starts_with('(') {
        let open = source.len() - scan.len();
        let close = find_matching_delimiter(source, open, '(', ')')?;
        scan = source[close + 1..].trim_start();
    }

    let name_paren_offset = scan.find('(')?;
    let params_open = source.len() - scan.len() + name_paren_offset;
    let params_close = find_matching_delimiter(source, params_open, '(', ')')?;
    let params = &source[params_open + 1..params_close];
    let suffix = source[params_close + 1..].trim_start();

    let mut total_slots = 0usize;
    let mut score_sum = 0.0;
    let mut pending_names = 0usize;

    for raw_part in split_top_level(params, ',') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        if !part.contains(' ') && !part.starts_with("...") {
            pending_names += 1;
            continue;
        }

        let Some(last_space) = part.rfind(' ') else {
            total_slots += 1 + pending_names;
            pending_names = 0;
            continue;
        };
        let names = part[..last_space]
            .split(',')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .count()
            + pending_names;
        let ty = part[last_space + 1..].trim();
        total_slots += names.max(1);
        score_sum += lang.type_token_score(ty) * names.max(1) as f64;
        pending_names = 0;
    }
    if pending_names > 0 {
        total_slots += pending_names;
    }

    total_slots += 1;
    if suffix.starts_with('{') {
        score_sum += 1.0;
    } else {
        let return_ty = take_until_top_level_char(suffix, &['{']).trim();
        if return_ty.is_empty() {
            score_sum += 1.0;
        } else {
            score_sum += lang.type_token_score(return_ty);
        }
    }

    Some(score_sum / total_slots as f64)
}

pub struct Go;

fn load() -> TsLanguage {
    tree_sitter_go::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "go",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: None,
};

fn go_root_package(specifier: &str) -> String {
    let parts: Vec<&str> = specifier.split('/').collect();
    if parts.len() <= 3 {
        if parts[0].contains('.') && parts.len() > 3 {
            parts[..3].join("/")
        } else {
            specifier.to_string()
        }
    } else if parts[0].contains('.') {
        parts[..3].join("/")
    } else {
        specifier.to_string()
    }
}

impl Language for Go {
    fn id(&self) -> LanguageId {
        GO
    }
    fn manifest(&self) -> &'static LanguageManifest {
        &GO_MANIFEST
    }
    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }

    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }
    fn root_package_from_specifier(&self, specifier: &str) -> String {
        go_root_package(specifier)
    }

    fn imports_query(&self) -> Option<&'static str> {
        Some(include_str!("queries/imports.scm"))
    }

    fn branch_keywords(&self) -> &'static [&'static str] {
        // Go has no `while` / `elif`; `switch` arms + `select` channel
        // arms use `case`.
        &["if", "else if", "for", "switch", "case", "select"]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &["import ", "package ", "func ", "type ", "var ", "const "]
    }
    fn is_test_symbol_id(&self, id: codesight_core::SymbolIdRef<'_>) -> bool {
        // Go convention: `TestXxx`, `BenchmarkXxx`, `FuzzXxx`, `ExampleXxx`
        // functions in `_test.go` files (file detection handled elsewhere).
        let name = id.leaf_name();
        name.starts_with("Test")
            || name.starts_with("Benchmark")
            || name.starts_with("Fuzz")
            || name.starts_with("Example")
    }

    fn display_import_lines(&self, edges: &[ImportEdge]) -> Vec<String> {
        // Go's conventional `import ( … )` block display — one line per spec
        // inside the block.
        if edges.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(edges.len() + 2);
        out.push("import (".to_string());
        for e in edges {
            out.push(format!("\t\"{}\"", e.specifier));
        }
        out.push(")".to_string());
        out
    }

    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let go_mod = dir.join("go.mod");
        if !go_mod.is_file() {
            return None;
        }
        let content = fs::read_to_string(&go_mod).ok()?;
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(module_path) = trimmed.strip_prefix("module ") {
                let module_path = module_path.trim();
                if !module_path.is_empty() {
                    return Some(module_path.to_string());
                }
            }
        }
        None
    }

    fn pruned_dirs(&self) -> &'static [&'static str] {
        &["vendor"]
    }

    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.go"]
    }
    fn is_test_file(&self, path: &str) -> bool {
        path.ends_with("_test.go")
    }

    fn is_restricted(&self, _source_text: &str, name: &str) -> bool {
        name.chars().next().map(|ch| !ch.is_uppercase()).unwrap_or(true)
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
    fn measured_type_density(&self, source: &str) -> Option<f64> {
        measure_go_signature(self, source)
    }
    fn type_token_score(&self, token: &str) -> f64 {
        let normalized = normalize_type_token(token);
        if normalized.is_empty() {
            return 0.0;
        }
        if matches!(normalized.as_str(), "any" | "interface{}") {
            return 0.0;
        }
        1.0
    }
}
