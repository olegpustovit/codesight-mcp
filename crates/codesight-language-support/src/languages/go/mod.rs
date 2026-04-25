//! Go language implementation.

use std::fs;
use std::path::Path;

use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::GO_MANIFEST;
use crate::manifest::{GO, GrammarSpec, LanguageId, LanguageManifest};
use crate::signature_density::{
    find_matching_delimiter, split_top_level, take_until_top_level_char,
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
    // Hostname-rooted module paths (github.com/owner/repo/…) collapse to the
    // first three segments; everything else is the module name as-is.
    if parts.len() > 3 && parts[0].contains('.') {
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
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        // Constructors `NewXxx` collapse to exact `New` only; method
        // names like `String` / `Error` / `ServeHTTP` satisfy stdlib
        // interfaces and repeat across types by design.
        &[
            "New",
            "String",
            "Error",
            "ServeHTTP",
            "Read",
            "Write",
            "Close",
            "Len",
            "Less",
            "Swap",
            "MarshalJSON",
            "UnmarshalJSON",
            "MarshalText",
            "UnmarshalText",
            "MarshalBinary",
            "UnmarshalBinary",
            "Format",
            "Scan",
            "Value",
            "Reset",
            "init",
            "main",
        ]
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
    fn test_file_suffixes(&self) -> &'static [&'static str] {
        &["_test.go"]
    }
    fn class_hierarchy_note(&self) -> Option<&'static str> {
        Some(
            "Go types do not use class inheritance. Interface \
             satisfaction is structural — use `inspect_graph` to trace \
             method and embedding relationships.",
        )
    }

    /// FIXUP T1.1 + crate_graph: interface-method dispatch lives here so
    /// `codesight-analysis` doesn't need a `.ends_with(".go")` branch.
    /// Returns `(interface_name, impl_files)` when `symbol_name` is part
    /// of an interface declared in `defining_source` (or in any indexed
    /// Go file when the defining file is itself an implementation site).
    /// Over-triggers — any file with a matching `func (recv) name(`
    /// receiver is treated as a candidate implementor.
    fn detect_trait_impl_sites(
        &self,
        defining_source: &str,
        repo_files: &[(String, String)],
        defining_path: &str,
        symbol_name: &str,
    ) -> Option<(String, Vec<String>)> {
        if symbol_name.is_empty() {
            return None;
        }
        let iface = owning_interface_for_method_go(defining_source, symbol_name).or_else(|| {
            repo_files
                .iter()
                .find_map(|(_, c)| owning_interface_for_method_go(c, symbol_name))
        });
        let iface = iface?;
        let mut hits = Vec::new();
        for (p, content) in repo_files {
            if p == defining_path {
                continue;
            }
            if !p.ends_with(".go") {
                continue;
            }
            if file_has_go_method_receiver(content, symbol_name) {
                hits.push(p.clone());
            }
        }
        hits.sort();
        hits.dedup();
        Some((iface, hits))
    }

    fn is_restricted(&self, _source_text: &str, name: &str) -> bool {
        name.chars()
            .next()
            .map(|ch| !ch.is_uppercase())
            .unwrap_or(true)
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
    fn weak_type_tokens(&self) -> &'static [&'static str] {
        &["any", "interface{}"]
    }
    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "break", "case", "chan", "const", "continue", "default", "defer", "else",
            "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map",
            "package", "range", "return", "select", "struct", "switch", "type", "var", "nil",
            "true", "false",
        ]
    }
}

// --- Interface-dispatch helpers (FIXUP T1.1; previously in
// codesight-analysis::impact). Kept private — only the trait method
// surfaces them.

fn owning_interface_for_method_go(source: &str, method: &str) -> Option<String> {
    let mut current_iface: Option<String> = None;
    let mut depth: i32 = 0;
    let method_prefix = format!("{method}(");
    for raw in source.lines() {
        let line = raw.trim_start();
        if current_iface.is_none() {
            if let Some(rest) = line.strip_prefix("type ")
                && let Some(idx) = rest.find(" interface")
            {
                let name = rest[..idx].trim_end_matches(&[' ', '[', '\t'][..]);
                let name = name
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or(name);
                if !name.is_empty() {
                    current_iface = Some(name.to_string());
                    depth = if line.contains('{') { 1 } else { 0 };
                }
            }
        } else {
            if line.starts_with('}') {
                depth -= 1;
                if depth <= 0 {
                    current_iface = None;
                    depth = 0;
                }
                continue;
            }
            if line.contains('{') {
                depth += 1;
            }
            if line.starts_with(&method_prefix)
                || line
                    .split_whitespace()
                    .next()
                    .map(|first| first == method || first.starts_with(&method_prefix))
                    .unwrap_or(false)
            {
                return current_iface.clone();
            }
        }
    }
    None
}

fn file_has_go_method_receiver(source: &str, method: &str) -> bool {
    let pat = format!(") {method}(");
    let pat_star = format!(") *{method}(");
    source
        .lines()
        .any(|line| line.contains(pat.as_str()) || line.contains(pat_star.as_str()))
}
