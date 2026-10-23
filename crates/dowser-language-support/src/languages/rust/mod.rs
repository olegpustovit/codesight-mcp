//! Rust language implementation.
//!
//! Carries Rust-specific liveness, visibility, and complexity heuristics that
//! used to live in `dowser-analysis::agent_readiness`,
//! `dowser-analysis::dead_code::entrypoints`, and `quality.rs`.

pub mod imports;
pub mod path_rename;
mod reachability;

use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::path::Path;

use dowser_core::symbols::FactQuality;
use tree_sitter::Language as TsLanguage;

use crate::family::{LanguageFamily, NestingMode};
use crate::imports_types::ImportEdge;
use crate::languages::builtins::RUST_MANIFEST;
use crate::manifest::{GrammarSpec, LanguageId, LanguageManifest, RUST};
use crate::signature_density::measure_colon_after_name;
use crate::traits::{ImportExtractionMode, Language};

/// Rust receiver detector — filters `self` / `&self` / `&mut self` from
/// parameter lists so they don't count against a function's type-annotation
/// density.
fn is_rust_receiver(param: &str) -> bool {
    let normalized = param.replace(' ', "");
    matches!(
        normalized.as_str(),
        "self" | "&self" | "&mutself" | "mutself" | "self:Self" | "self:Box<Self>"
    ) || normalized.starts_with("self:")
}

// Rust overrides `extract_imports` directly with a hand-rolled walker —
// the nested `use a::{b::C, d}` tree doesn't flatten cleanly through SCM
// capture grouping, so we keep a bespoke flattener in `imports.rs`.

pub struct Rust;

fn load() -> TsLanguage {
    tree_sitter_rust::LANGUAGE.into()
}

pub static GRAMMAR: GrammarSpec = GrammarSpec {
    grammar_name: "rust",
    loader: load,
    tags_scm: include_str!("queries/tags.scm"),
    injections_scm: Some(include_str!("queries/injections.scm")),
};

impl Language for Rust {
    fn id(&self) -> LanguageId {
        RUST
    }

    fn manifest(&self) -> &'static LanguageManifest {
        &RUST_MANIFEST
    }

    fn grammar_spec(&self) -> Option<&'static GrammarSpec> {
        Some(&GRAMMAR)
    }

    // ── Imports ─────────────────────────────────────────────────────────
    fn extraction_mode(&self) -> Option<ImportExtractionMode> {
        Some(ImportExtractionMode::TreeSitterFirst)
    }

    fn root_package_from_specifier(&self, specifier: &str) -> String {
        specifier.split("::").next().unwrap_or("").to_string()
    }

    fn extract_imports(&self, source: &str) -> Vec<ImportEdge> {
        imports::extract(source)
    }

    fn reachability_edges(
        &self,
        content: &str,
        source_file: &str,
    ) -> Vec<dowser_core::StructuralEdge> {
        reachability::reachability_edges(content, source_file)
    }

    /// Rust-idiomatic `use` rendering (ROADMAP P1.6). Single name →
    /// `use spec::name;`, multiple → `use spec::{a, b};`, empty names →
    /// `use spec;`. Overrides the trait's default ES-module shape.
    fn display_import_lines(&self, edges: &[ImportEdge]) -> Vec<String> {
        let mut lines = Vec::new();
        for e in edges {
            let spec = e.specifier.trim_matches(':');
            let line = match e.names.as_slice() {
                [] if spec.is_empty() => continue,
                [] => format!("use {spec};"),
                [n] if spec.is_empty() => format!("use {n};"),
                [n] => format!("use {spec}::{n};"),
                many if spec.is_empty() => format!("use {{{}}};", many.join(", ")),
                many => format!("use {spec}::{{{}}};", many.join(", ")),
            };
            lines.push(line);
        }
        lines
    }

    fn resolve_import_edge(
        &self,
        edge: &ImportEdge,
        from_file: &str,
        _source_files: &FxHashSet<String>,
        known_files_vec: &[String],
        _alias_map: Option<&FxHashMap<String, Vec<String>>>,
        workspace_crates: Option<&FxHashMap<String, String>>,
    ) -> Option<(String, FactQuality)> {
        imports::resolve_edge(edge, from_file, known_files_vec, workspace_crates)
            .map(|path| (path, FactQuality::BaselineStructural))
    }

    fn branch_keywords(&self) -> &'static [&'static str] {
        // `match` is the main decision construct in Rust; `if let` /
        // `while let` share `if` / `while`. Each `=>` arm on a line bumps
        // complexity via the operator sweep.
        &["if", "else if", "for", "while", "loop", "match"]
    }
    fn idiomatic_symbol_names(&self) -> &'static [&'static str] {
        &[
            "new",
            "default",
            "from",
            "into",
            "try_from",
            "try_into",
            "as_ref",
            "as_mut",
            "as_str",
            "as_bytes",
            "clone",
            "eq",
            "ne",
            "cmp",
            "partial_cmp",
            "hash",
            "fmt",
            "drop",
            "deref",
            "deref_mut",
            "len",
            "is_empty",
            "iter",
            "iter_mut",
            "into_iter",
            "next",
            "build",
            "builder",
            "push",
            "pop",
            "get",
            "get_mut",
            "insert",
            "remove",
            "contains",
            "index",
            "index_mut",
            "borrow",
            "borrow_mut",
            "to_string",
            "to_owned",
        ]
    }
    fn branch_operators(&self) -> &'static [&'static str] {
        &["&&", "||", "=>"]
    }
    fn line_comment_prefixes(&self) -> &'static [&'static str] {
        &["//"]
    }
    fn declaration_line_prefixes(&self) -> &'static [&'static str] {
        &[
            "use ",
            "pub use ",
            "mod ",
            "pub mod ",
            "fn ",
            "pub fn ",
            "struct ",
            "pub struct ",
            "enum ",
            "pub enum ",
            "trait ",
            "pub trait ",
            "impl ",
            "type ",
            "const ",
            "static ",
        ]
    }
    fn is_test_symbol_id(&self, id: dowser_core::SymbolIdRef<'_>) -> bool {
        // Rust inline tests: `tests` parent scope (most common) or `#[test]`
        // functions named `test_…` at any depth.
        id.has_scope("tests") || id.leaf_name().starts_with("test_")
    }
    fn class_hierarchy_note(&self) -> Option<&'static str> {
        Some(
            "Rust structs do not use class inheritance. Trait impls and \
             trait bounds live in a separate graph — use `inspect_graph \
             graph=references` or `graph=related` against the target symbol \
             instead.",
        )
    }

    // ── Packages ───────────────────────────────────────────────────────
    fn package_name_for_dir(&self, dir: &Path) -> Option<String> {
        let cargo_toml = dir.join("Cargo.toml");
        if !cargo_toml.is_file() {
            return None;
        }
        let content = fs::read_to_string(&cargo_toml).ok()?;
        let table: toml::Value = content.parse().ok()?;
        table
            .get("package")?
            .get("name")?
            .as_str()
            .map(str::to_string)
    }

    fn pruned_dirs(&self) -> &'static [&'static str] {
        &["target"]
    }

    fn vendor_segments(&self) -> &'static [&'static str] {
        &["target"]
    }

    fn reserved_identifiers(&self) -> &'static [&'static str] {
        &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "type", "union", "unsafe", "use", "where", "while", "box", "do",
            "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield",
        ]
    }

    fn outline_noise_basenames(&self) -> &'static [&'static str] {
        &["build.rs", ".dowser.toml", ".mcp.json"]
    }

    fn barrel_file_basenames(&self) -> &'static [&'static str] {
        &["mod.rs", "lib.rs"]
    }

    // ── Liveness ───────────────────────────────────────────────────────
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.rs", "lib.rs", "build.rs"]
    }

    fn test_file_suffixes(&self) -> &'static [&'static str] {
        &["_test.rs"]
    }

    fn test_file_dirs(&self) -> &'static [&'static str] {
        &["/tests/", "/test/"]
    }

    /// Cargo integration-test convention: a `.rs` file that lives directly
    /// under a `tests/` directory (workspace-root or per-crate). Each one
    /// links as its own test binary; nested `tests/foo/helper.rs` modules
    /// don't count as roots.
    fn is_test_target_root(&self, path: &str) -> bool {
        if !path.ends_with(".rs") {
            return false;
        }
        if let Some(rest) = path.strip_prefix("tests/") {
            return !rest.contains('/');
        }
        if let Some(idx) = path.rfind("/tests/") {
            let rest = &path[idx + 7..];
            return !rest.contains('/');
        }
        false
    }

    /// insta snapshots: `.snap` is the committed baseline, `.snap.new` is the
    /// most recent divergence awaiting review, `.pending.snap` is the
    /// current-run candidate that has not been accepted yet.
    fn test_fixture_suffixes(&self) -> &'static [&'static str] {
        &[".snap", ".snap.new", ".pending.snap"]
    }

    fn test_fixture_dirs(&self) -> &'static [&'static str] {
        &["/fixtures/"]
    }

    fn is_conventional_path(&self, path: &str) -> bool {
        CONVENTIONAL_RUST_DIRS
            .iter()
            .any(|dir| path.starts_with(dir) || path.contains(&format!("/{dir}")))
            || path.rsplit('/').next().unwrap_or(path) == "build.rs"
    }

    fn conventional_dirs(&self) -> &'static [&'static str] {
        CONVENTIONAL_RUST_DIRS
    }

    fn path_rename_engine(&self) -> Option<&'static dyn crate::path_rename::PathRenameEngine> {
        Some(&path_rename::RUST_PATH_RENAME_ENGINE)
    }

    fn additional_live_roots(
        &self,
        root: Option<&Path>,
        all_files: &[String],
    ) -> Vec<crate::traits::LiveRoot> {
        use crate::traits::{LiveRoot, LiveRootConfidence};
        let mut roots = Vec::new();
        let cargo_targets = detect_cargo_target_files(root, all_files);
        for path in &cargo_targets {
            let message =
                format!("file '{path}' is a Cargo [[test]]/[[example]]/[[bench]]/[[bin]] target");
            roots.push(LiveRoot {
                path: path.clone(),
                reason: "cargo_target",
                message,
                confidence: LiveRootConfidence::Structural,
                explicit: true,
            });
        }
        // Tier 4.1: any module-prefixed call in a bin entry's body
        // (`hooks::run_pretooluse()`, `cli::serve()`, …) implies the host
        // module is reachable at runtime even if no static `use` line
        // mentions the callee — common pattern in clap subcommand
        // dispatch. Promote those host files to live roots.
        for path in detect_dispatch_handler_files(root, all_files, &cargo_targets) {
            let message = format!(
                "file '{path}' is the host module of a path-call expression in a bin entry (e.g. `module::handler()` from main.rs)"
            );
            roots.push(LiveRoot {
                path,
                reason: "bin_entry_dispatch",
                message,
                confidence: LiveRootConfidence::Heuristic,
                explicit: false,
            });
        }
        for path in detect_foreign_trait_impl_files(root, all_files) {
            let message = format!(
                "file '{path}' contains `impl <ForeignTrait> for X` — reached at runtime via vtable"
            );
            roots.push(LiveRoot {
                path,
                reason: "foreign_trait_impl",
                message,
                confidence: LiveRootConfidence::Heuristic,
                explicit: false,
            });
        }
        roots
    }

    // ── Visibility ─────────────────────────────────────────────────────
    fn is_restricted(&self, source_text: &str, _name: &str) -> bool {
        let trimmed = source_text.trim_start();
        if trimmed.starts_with("pub(crate)")
            || trimmed.starts_with("pub(super)")
            || trimmed.starts_with("pub(self)")
            || trimmed.starts_with("pub(in ")
        {
            return true;
        }
        !trimmed.starts_with("pub ")
    }

    // ── Complexity ─────────────────────────────────────────────────────
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
        measure_colon_after_name(source, self, true, is_rust_receiver)
    }
    /// Rust's escape hatches: `dyn Any` / `Box<dyn Any>` / `impl Any`.
    fn weak_type_substrings(&self) -> &'static [&'static str] {
        &["<any>", "dynany"]
    }

    /// FIXUP T1.1 + crate_graph: trait-method dispatch detection lives
    /// here so `dowser-analysis` doesn't need a `.ends_with(".rs")`
    /// branch. Returns `(trait_name, impl_files)` when `symbol_name`
    /// belongs to a trait declared in `defining_source` (or in any
    /// indexed Rust file when the defining file is itself an `impl`
    /// site).
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
        let trait_owner = owning_trait_for_symbol(defining_source, symbol_name).or_else(|| {
            owning_trait_from_impl_site(defining_source, symbol_name).and_then(|candidate| {
                repo_files.iter().find_map(|(_, c)| {
                    owning_trait_for_symbol(c, symbol_name).filter(|found| found == &candidate)
                })
            })
        });
        let trait_owner = trait_owner?;
        let mut hits = Vec::new();
        for (p, content) in repo_files {
            if p == defining_path {
                continue;
            }
            if !p.ends_with(".rs") {
                continue;
            }
            if file_impls_trait_method(content, &trait_owner, symbol_name)
                || owning_trait_for_symbol(content, symbol_name).as_deref()
                    == Some(trait_owner.as_str())
            {
                hits.push(p.clone());
            }
        }
        hits.sort();
        hits.dedup();
        Some((trait_owner, hits))
    }
}

// --- Trait-dispatch helpers (FIXUP T1.1; previously in
// dowser-analysis::impact). Kept private — only the trait method
// surfaces them.

fn owning_trait_for_symbol(source: &str, symbol_name: &str) -> Option<String> {
    let needle = format!("fn {symbol_name}");
    let mut in_trait: Option<String> = None;
    let mut depth: i32 = 0;
    for raw in source.lines() {
        let line = raw.trim_start();
        if in_trait.is_none() {
            if let Some(name) = parse_trait_header(line) {
                in_trait = Some(name);
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_trait = None;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&needle)
                && line
                    .split(&needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return in_trait;
            }
            if depth <= 0 {
                in_trait = None;
            }
        }
    }
    None
}

fn parse_trait_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("pub ")
        .or_else(|| line.strip_prefix("pub(crate) "))
        .or_else(|| line.strip_prefix("pub(super) "))
        .or_else(|| line.strip_prefix("unsafe "))
        .unwrap_or(line);
    let rest = rest.strip_prefix("trait ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn file_impls_trait_method(source: &str, trait_name: &str, symbol_name: &str) -> bool {
    let impl_prefix_a = format!("impl {trait_name} for ");
    let impl_prefix_b = "impl<".to_string();
    let fn_needle = format!("fn {symbol_name}");
    let mut in_impl = false;
    let mut depth: i32 = 0;
    for raw in source.lines() {
        let line = raw.trim_start();
        if !in_impl {
            if line.starts_with(&impl_prefix_a)
                || (line.starts_with(&impl_prefix_b)
                    && line.contains(&format!(" {trait_name} for ")))
            {
                in_impl = true;
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_impl = false;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&fn_needle)
                && line
                    .split(&fn_needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return true;
            }
            if depth <= 0 {
                in_impl = false;
            }
        }
    }
    false
}

fn owning_trait_from_impl_site(source: &str, symbol_name: &str) -> Option<String> {
    let fn_needle = format!("fn {symbol_name}");
    let mut current_trait: Option<String> = None;
    let mut depth: i32 = 0;
    let mut in_impl = false;
    for raw in source.lines() {
        let line = raw.trim_start();
        if !in_impl {
            if let Some(name) = parse_impl_trait_header(line) {
                current_trait = Some(name);
                in_impl = true;
                depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                if depth <= 0 {
                    in_impl = false;
                    current_trait = None;
                }
                continue;
            }
        } else {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if line.contains(&fn_needle)
                && line
                    .split(&fn_needle)
                    .nth(1)
                    .is_some_and(|tail| matches!(tail.chars().next(), Some('(') | Some('<')))
            {
                return current_trait;
            }
            if depth <= 0 {
                in_impl = false;
                current_trait = None;
            }
        }
    }
    None
}

fn parse_impl_trait_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("unsafe ")
        .unwrap_or(line)
        .strip_prefix("impl")?
        .trim_start();
    let rest = if let Some(r) = rest.strip_prefix('<') {
        let mut depth = 1i32;
        let mut idx = 0usize;
        for (i, c) in r.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        idx = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        r[idx..].trim_start()
    } else {
        rest
    };
    let for_pos = rest.find(" for ")?;
    let trait_part = rest[..for_pos].trim();
    let name: String = trait_part
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// Map every workspace-member crate name (underscore form, as used in `use`
/// statements) to the repo-relative path of its library root (`src/lib.rs` by
/// default, or the `[lib] path` override when set).
pub fn workspace_crates_from_root(root: &Path) -> FxHashMap<String, String> {
    let mut out = FxHashMap::default();
    if !root.is_dir() {
        return out;
    }
    let walker = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .hidden(false)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if entry.file_name() != "Cargo.toml" {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(parsed) = content.parse::<toml::Value>() else {
            continue;
        };
        let Some(name) = parsed
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        let Some(manifest_dir) = entry.path().parent() else {
            continue;
        };
        let lib_rel = parsed
            .get("lib")
            .and_then(|l| l.get("path"))
            .and_then(|p| p.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "src/lib.rs".to_string());
        let lib_abs = manifest_dir.join(&lib_rel);
        if !lib_abs.is_file() {
            continue;
        }
        let Ok(relative) = lib_abs.strip_prefix(root) else {
            continue;
        };
        let repo_rel = relative.to_string_lossy().replace('\\', "/");
        out.insert(name.replace('-', "_"), repo_rel);
    }
    out
}

/// Same as [`workspace_crates_from_root`] but also returns `src/bin/*.rs`
/// files — implicit `[[bin]]` targets that cargo auto-discovers.
pub fn workspace_bin_targets_from_root(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let walker = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .hidden(false)
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        if entry.file_name() != "Cargo.toml" {
            continue;
        }
        let Some(manifest_dir) = entry.path().parent() else {
            continue;
        };
        let bin_dir = manifest_dir.join("src/bin");
        if !bin_dir.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&bin_dir) else {
            continue;
        };
        for file in entries.flatten() {
            let path = file.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if let Ok(relative) = path.strip_prefix(root) {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

/// Foreign-trait implementations that mark their enclosing file as live
/// regardless of direct-reference edges.
pub const FOREIGN_TRAIT_BASES: &[&str] = &[
    "Drop",
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "Display",
    "Hash",
    "Eq",
    "PartialEq",
    "Ord",
    "PartialOrd",
    "From",
    "Into",
    "AsRef",
    "AsMut",
    "TryFrom",
    "TryInto",
    "Iterator",
    "IntoIterator",
    "Error",
    "Send",
    "Sync",
    "Future",
    "Serialize",
    "Deserialize",
    "Dispatcher",
    "CodeBackend",
];

const CONVENTIONAL_RUST_DIRS: &[&str] = &[
    "scripts/",
    "tests/",
    "examples/",
    "benches/",
    "fixtures/",
    "testdata/",
];

fn detect_foreign_trait_impl_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> FxHashSet<String> {
    let mut hits = FxHashSet::default();
    let Some(root) = root_path else {
        return hits;
    };
    let alt = FOREIGN_TRAIT_BASES.join("|");
    let pattern = format!(r"\bimpl\b[^{{;]*?\b(?:{alt})\b(?:<[^>]*>)?\s+for\b");
    let Ok(re) = regex::Regex::new(&pattern) else {
        return hits;
    };
    for file in all_files {
        if !file.ends_with(".rs") {
            continue;
        }
        let abs = root.join(file);
        let Ok(content) = fs::read_to_string(&abs) else {
            continue;
        };
        if re.is_match(&content) {
            hits.insert(file.clone());
        }
    }
    hits
}

fn detect_cargo_target_files(root_path: Option<&Path>, all_files: &[String]) -> FxHashSet<String> {
    let mut targets = FxHashSet::default();
    let Some(root) = root_path else {
        return targets;
    };

    let file_set: FxHashSet<&str> = all_files.iter().map(String::as_str).collect();
    let cargo_dirs: Vec<String> = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .hidden(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_some_and(|ft| ft.is_file()) && entry.file_name() == "Cargo.toml"
        })
        .filter_map(|entry| {
            entry
                .path()
                .parent()
                .and_then(|p| p.strip_prefix(root).ok())
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"))
        })
        .collect();

    let join_rel = |dir: &str, sub: &str| -> String {
        if dir.is_empty() {
            sub.to_string()
        } else {
            format!("{dir}/{sub}")
        }
    };

    for cargo_dir in &cargo_dirs {
        for subdir in ["tests", "examples", "benches", "src/bin"] {
            let prefix = join_rel(cargo_dir, subdir);
            for file in all_files {
                if !file.ends_with(".rs") {
                    continue;
                }
                let matches_subdir = file == &prefix
                    || file.starts_with(&format!("{prefix}/"))
                    || file.starts_with(&format!("{prefix}\\"));
                if matches_subdir {
                    targets.insert(file.clone());
                }
            }
        }

        let cargo_toml_rel = join_rel(cargo_dir, "Cargo.toml");
        let cargo_toml_abs = root.join(&cargo_toml_rel);
        let Ok(contents) = fs::read_to_string(&cargo_toml_abs) else {
            continue;
        };
        let Ok(parsed) = contents.parse::<toml::Value>() else {
            continue;
        };
        for key in ["test", "example", "bench", "bin"] {
            let Some(entries) = parsed.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for entry in entries {
                let Some(path) = entry.get("path").and_then(|p| p.as_str()) else {
                    continue;
                };
                let rel = join_rel(cargo_dir, path);
                if file_set.contains(rel.as_str()) {
                    targets.insert(rel);
                }
            }
        }
    }

    targets
}

/// Tier 4.1: scan bin-entry files for `<module>::<fn>(` call expressions
/// and resolve each `<module>` to a sibling file in the same crate's `src/`
/// dir. Those host files are promoted to live roots so dead-code analysis
/// doesn't flag clap subcommand handlers (e.g. `hooks::run_pretooluse()`
/// from `src/main.rs` → `src/cli/hooks.rs` or `src/hooks.rs`).
///
/// Conservative: only fires on lowercase module names (Rust convention),
/// requires `(` immediately after the function ident, and resolves only
/// to files already in `all_files`. Skips path calls whose first segment
/// matches a `use` alias (those are direct calls, already covered by the
/// import edge graph).
fn detect_dispatch_handler_files(
    root_path: Option<&Path>,
    all_files: &[String],
    bin_entries: &FxHashSet<String>,
) -> FxHashSet<String> {
    let mut handlers = FxHashSet::default();
    let Some(root) = root_path else {
        return handlers;
    };
    if bin_entries.is_empty() {
        return handlers;
    }
    let file_set: FxHashSet<&str> = all_files.iter().map(String::as_str).collect();
    for bin in bin_entries {
        let abs = root.join(bin);
        let Ok(source) = fs::read_to_string(&abs) else {
            continue;
        };
        // Sibling-search dir: `<bin_parent>/src/` if `<bin>` is `src/main.rs`,
        // else the bin's parent directory itself (e.g. `crates/foo/src/main.rs`
        // → `crates/foo/src/`).
        let bin_parent = std::path::Path::new(bin)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        for module in scan_module_path_calls(&source) {
            for candidate in resolve_module_to_file(bin_parent, &module, &file_set) {
                handlers.insert(candidate);
            }
        }
    }
    handlers
}

/// Find lowercase-module path-call expressions (`module::fn(`) in `source`.
/// Returns the module ident only — the function name doesn't matter for
/// reachability of the host file.
fn scan_module_path_calls(source: &str) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        // Look for `::`.
        if bytes[i] != b':' || bytes[i + 1] != b':' {
            i += 1;
            continue;
        }
        // Walk back to find the module ident.
        let mod_end = i;
        let mut mod_start = mod_end;
        while mod_start > 0 {
            let b = bytes[mod_start - 1];
            if b.is_ascii_alphanumeric() || b == b'_' {
                mod_start -= 1;
            } else {
                break;
            }
        }
        if mod_start == mod_end {
            i += 2;
            continue;
        }
        // Walk forward over the function ident.
        let fn_start = i + 2;
        let mut fn_end = fn_start;
        while fn_end < bytes.len() {
            let b = bytes[fn_end];
            if b.is_ascii_alphanumeric() || b == b'_' {
                fn_end += 1;
            } else {
                break;
            }
        }
        if fn_end == fn_start {
            i = fn_start;
            continue;
        }
        // Skip whitespace, then expect `(`.
        let mut p = fn_end;
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        if p < bytes.len() && bytes[p] == b'(' {
            // ASCII slice safe because the bytes are alnum/underscore.
            let module = std::str::from_utf8(&bytes[mod_start..mod_end]).unwrap_or("");
            // Lowercase first char filters out types (Self, Vec, Foo) —
            // Rust convention reserves snake_case for modules.
            if module
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
                && module != "self"
                && module != "super"
                && module != "crate"
            {
                out.insert(module.to_string());
            }
        }
        i = fn_end;
    }
    out
}

/// Map a module ident to candidate sibling-file paths. For `bin_parent =
/// "src"` and `module = "hooks"`, returns `["src/hooks.rs",
/// "src/hooks/mod.rs", "src/cli/hooks.rs", "src/cli/hooks/mod.rs"]` filtered
/// to files actually in `all_files`.
fn resolve_module_to_file(
    bin_parent: &str,
    module: &str,
    file_set: &FxHashSet<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let prefix = if bin_parent.is_empty() {
        String::new()
    } else {
        format!("{bin_parent}/")
    };
    let candidates = [
        format!("{prefix}{module}.rs"),
        format!("{prefix}{module}/mod.rs"),
        // Common nesting: `src/cli/<module>.rs`.
        format!("{prefix}cli/{module}.rs"),
        format!("{prefix}cli/{module}/mod.rs"),
        format!("{prefix}commands/{module}.rs"),
        format!("{prefix}commands/{module}/mod.rs"),
    ];
    for c in candidates {
        if file_set.contains(c.as_str()) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_is_test_fixture_detects_insta_snapshot_extensions() {
        let rust = Rust;
        assert!(rust.is_test_fixture("src/tests/__snapshots__/greet.snap"));
        assert!(rust.is_test_fixture("src/tests/greet.snap.new"));
        assert!(rust.is_test_fixture("src/tests/greet.pending.snap"));
        // fixtures/ keeps working.
        assert!(rust.is_test_fixture("fixtures/hello.rs"));
        assert!(rust.is_test_fixture("tests/fixtures/lang_samples/sample.rs"));
        // Ordinary source files are not fixtures.
        assert!(!rust.is_test_fixture("src/lib.rs"));
    }

    #[test]
    fn workspace_crates_dash_becomes_underscore_and_points_at_lib_rs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("crates/my-lib/src")).unwrap();
        fs::write(
            root.join("crates/my-lib/Cargo.toml"),
            "[package]\nname = \"my-lib\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/my-lib/src/lib.rs"), "").unwrap();
        let map = workspace_crates_from_root(root);
        assert_eq!(
            map.get("my_lib").map(String::as_str),
            Some("crates/my-lib/src/lib.rs")
        );
    }

    #[test]
    fn workspace_crates_skips_binary_only_crate_without_lib_rs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("crates/cli-only/src")).unwrap();
        fs::write(
            root.join("crates/cli-only/Cargo.toml"),
            "[package]\nname = \"cli-only\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/cli-only/src/main.rs"), "").unwrap();
        let map = workspace_crates_from_root(root);
        assert!(!map.contains_key("cli_only"));
    }

    #[test]
    fn workspace_bin_targets_collects_src_bin_rs_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("src/bin/a.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/bin/b.rs"), "fn main() {}").unwrap();
        let bins = workspace_bin_targets_from_root(root);
        assert!(bins.iter().any(|p| p == "src/bin/a.rs"));
        assert!(bins.iter().any(|p| p == "src/bin/b.rs"));
    }

    #[test]
    fn package_name_for_dir_reads_cargo_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(
            Rust.package_name_for_dir(tmp.path()),
            Some("my-crate".into())
        );
    }
}
