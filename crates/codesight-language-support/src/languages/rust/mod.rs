//! Rust language implementation.
//!
//! Carries Rust-specific liveness, visibility, and complexity heuristics that
//! used to live in `codesight-analysis::agent_readiness`,
//! `codesight-analysis::dead_code::entrypoints`, and `quality.rs`.

pub mod imports;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

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

    // ── Liveness ───────────────────────────────────────────────────────
    fn entrypoint_basenames(&self) -> &'static [&'static str] {
        &["main.rs", "lib.rs", "build.rs"]
    }

    fn is_test_file(&self, path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with("_test.rs")
            || lower.contains("/tests/")
            || lower.starts_with("tests/")
            || lower.contains("/test/")
            || lower.starts_with("test/")
    }

    fn is_test_fixture(&self, path: &str) -> bool {
        path.contains("/fixtures/")
            || path.starts_with("fixtures/")
            || path.contains("/lang_samples/")
    }

    fn is_conventional_path(&self, path: &str) -> bool {
        CONVENTIONAL_RUST_DIRS.iter().any(|dir| {
            path.starts_with(dir) || path.contains(&format!("/{dir}"))
        }) || path.rsplit('/').next().unwrap_or(path) == "build.rs"
    }

    fn additional_live_root_files(
        &self,
        root: Option<&Path>,
        all_files: &[String],
    ) -> Vec<String> {
        let mut roots = Vec::new();
        roots.extend(detect_foreign_trait_impl_files(root, all_files));
        roots.extend(detect_cargo_target_files(root, all_files));
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
        measure_colon_after_name(source, "rust", true, is_rust_receiver)
    }
}

/// Map every workspace-member crate name (underscore form, as used in `use`
/// statements) to the repo-relative path of its library root (`src/lib.rs` by
/// default, or the `[lib] path` override when set).
pub fn workspace_crates_from_root(root: &Path) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
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
    "stress/",
];

fn detect_foreign_trait_impl_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> HashSet<String> {
    let mut hits = HashSet::new();
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

fn detect_cargo_target_files(
    root_path: Option<&Path>,
    all_files: &[String],
) -> HashSet<String> {
    let mut targets = HashSet::new();
    let Some(root) = root_path else {
        return targets;
    };

    let file_set: HashSet<&str> = all_files.iter().map(String::as_str).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_crates_dash_becomes_underscore_and_points_at_lib_rs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("crates/mcp-rpc/src")).unwrap();
        fs::write(
            root.join("crates/mcp-rpc/Cargo.toml"),
            "[package]\nname = \"mcp-rpc\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(root.join("crates/mcp-rpc/src/lib.rs"), "").unwrap();
        let map = workspace_crates_from_root(root);
        assert_eq!(
            map.get("mcp_rpc").map(String::as_str),
            Some("crates/mcp-rpc/src/lib.rs")
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
        assert_eq!(Rust.package_name_for_dir(tmp.path()), Some("my-crate".into()));
    }
}
