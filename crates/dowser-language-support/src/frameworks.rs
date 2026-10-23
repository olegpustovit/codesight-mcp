//! Framework detection, context providers, enrichment, and shared helpers.
//!
//! Per LANGUAGES.md §3.6 framework-specific code now lives under the host
//! language folder (`languages/typescript/nextjs/`, `languages/vue/nuxt/`,
//! `languages/php/laravel/`, `languages/sql/dbt/`). This module owns the
//! shared trait, registry, payload types, and profile helpers that those
//! per-framework modules plug into.

use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

pub mod profiles;

// ── Shared walk helpers ───────────────────────────────────────────────────

/// Recursively walk files under `dir`, yielding each regular file's path.
/// Replaces hand-rolled `read_dir` + recurse loops across framework snapshots.
pub fn walk_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
}

/// Produce a forward-slash-normalized repo-relative path, or `None` on
/// prefix mismatch. Centralises the `strip_prefix` + `replace('\\', "/")`
/// pattern that was duplicated ~8 times across framework snapshots.
pub fn rel_path(path: &Path, root: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

/// Parse `source` with the given tree-sitter grammar, returning the parse
/// tree. Used by Next.js route-handler extraction; inlined here so the
/// language-support crate does not need an upward dep on extractors.
pub fn parse_tree(
    lang: impl Into<tree_sitter::Language>,
    source: &str,
) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang.into()).ok()?;
    parser.parse(source, None)
}

// ── FileContext ────────────────────────────────────────────────────────────

/// Metadata a provider attaches to a specific file.
#[derive(Debug, Clone, Default)]
pub struct FileContext {
    pub description: String,
    pub tags: Vec<String>,
    /// Ordered map of property name -> optional description.
    pub properties: Vec<(String, String)>,
}

impl FileContext {
    /// Rich multiline context string for symbol enrichment.
    pub fn summary_context(&self) -> String {
        self.summary_context_with_max(10)
    }

    pub fn summary_context_with_max(&self, max_properties: usize) -> String {
        let mut parts: Vec<String> = Vec::new();

        if !self.description.is_empty() {
            parts.push(self.description.clone());
        }
        if !self.tags.is_empty() {
            parts.push(self.tags.join(", "));
        }
        if !self.properties.is_empty() {
            let shown: Vec<String> = self
                .properties
                .iter()
                .take(max_properties)
                .map(|(k, v)| {
                    if v.is_empty() {
                        k.clone()
                    } else {
                        format!("{k} ({v})")
                    }
                })
                .collect();
            let mut prop_str = shown.join(", ");
            let remaining = self.properties.len().saturating_sub(max_properties);
            if remaining > 0 {
                prop_str.push_str(&format!("... and {remaining} more"));
            }
            parts.push(prop_str);
        }
        parts.join("\n")
    }

    /// One-line summary for file-level display.
    pub fn file_summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        if !self.description.is_empty() {
            let desc = if self.description.len() > 200 {
                format!("{}...", &self.description[..197])
            } else {
                self.description.clone()
            };
            parts.push(desc);
        }
        if !self.tags.is_empty() {
            parts.push(format!("Tags: {}", self.tags.join(", ")));
        }
        if !self.properties.is_empty() {
            parts.push(format!("{} properties", self.properties.len()));
        }
        parts.join(", ")
    }

    /// Keywords for search indexing: tags + property names.
    pub fn search_keywords(&self) -> Vec<String> {
        let mut kw: Vec<String> = self.tags.clone();
        for (name, _) in &self.properties {
            kw.push(name.clone());
        }
        kw
    }
}

// ── ExtraImport ────────────────────────────────────────────────────────────

/// An extra import edge a provider wants added to the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtraImport {
    pub specifier: String,
    pub names: Vec<String>,
}

// ── Framework provider macro ──────────────────────────────────────────────

/// Generate a framework provider struct + `FrameworkProvider` impl from only
/// the parts that differ: name, detect, load, and optional extra_imports.
/// Common boilerplate (struct, new, stats, Default, get_file_context,
/// get_metadata) is emitted by the macro.
///
/// Each invocation site must have `render_file_context` and `metadata` in
/// scope (via `use super::render::{metadata, render_file_context}`).
#[macro_export]
macro_rules! impl_framework_provider {
    (
        provider: $Provider:ident,
        snapshot: $Snapshot:ty,
        stats: $Stats:ty,
        name: $name:expr,
        detect($det_root:ident) $detect_body:block,
        load($load_self:ident, $load_root:ident) $load_body:block
        $(, extra_imports($ei_self:ident) $ei_body:block)?
    ) => {
        pub struct $Provider {
            snapshot: Option<$Snapshot>,
        }

        impl $Provider {
            pub fn new() -> Self {
                Self { snapshot: None }
            }

            pub fn stats(&self) -> $Stats {
                self.snapshot
                    .as_ref()
                    .map(<$Snapshot>::stats)
                    .unwrap_or_default()
            }
        }

        impl Default for $Provider {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $crate::frameworks::FrameworkProvider for $Provider {
            fn name(&self) -> &str {
                $name
            }

            fn detect(&self, $det_root: &std::path::Path) -> bool {
                $detect_body
            }

            fn load(&mut $load_self, $load_root: &std::path::Path) -> anyhow::Result<()> {
                $load_body
            }

            fn get_file_context(&self, rel_path: &str) -> Option<$crate::frameworks::FileContext> {
                self.snapshot
                    .as_ref()
                    .and_then(|snapshot| render_file_context(snapshot, rel_path))
            }

            fn get_metadata(
                &self,
            ) -> rustc_hash::FxHashMap<String, serde_json::Value> {
                self.snapshot.as_ref().map(metadata).unwrap_or_default()
            }

            $(
                fn get_extra_imports(
                    &$ei_self,
                ) -> anyhow::Result<
                    rustc_hash::FxHashMap<String, Vec<$crate::frameworks::ExtraImport>>,
                > {
                    $ei_body
                }
            )?
        }
    };
}

// ── FrameworkProvider trait ─────────────────────────────────────────────────

/// A framework-aware context provider.
///
/// Concrete implementations live in `languages/<host>/<framework>/`.
pub trait FrameworkProvider: Send + Sync {
    /// Human-readable name (e.g. "dbt", "nextjs").
    fn name(&self) -> &str;

    /// Check whether this framework is present in `root`.
    fn detect(&self, root: &Path) -> bool;

    /// Load / parse project metadata. May fail; errors are caught by the
    /// registry so a single failing provider cannot crash discovery.
    fn load(&mut self, root: &Path) -> anyhow::Result<()>;

    /// Return file-level context for a path relative to the project root.
    fn get_file_context(&self, rel_path: &str) -> Option<FileContext>;

    /// Return extra metadata key-value pairs for the project.
    fn get_metadata(&self) -> FxHashMap<String, serde_json::Value> {
        FxHashMap::default()
    }

    /// Return extra imports keyed by relative file path.
    fn get_extra_imports(&self) -> anyhow::Result<FxHashMap<String, Vec<ExtraImport>>> {
        Ok(FxHashMap::default())
    }
}

// ── Symbol enrichment helpers ──────────────────────────────────────────────

/// A minimal symbol representation for enrichment.
#[derive(Debug, Clone, Default)]
pub struct EnrichableSymbol {
    pub file: String,
    pub ecosystem_context: String,
    pub keywords: Vec<String>,
}

/// Enrich a symbol using all active providers.
pub fn enrich_symbol(symbol: &mut EnrichableSymbol, providers: &[Box<dyn FrameworkProvider>]) {
    let mut context_parts: Vec<String> = Vec::new();

    for p in providers {
        if let Some(ctx) = p.get_file_context(&symbol.file) {
            context_parts.push(format!("[{}] {}", p.name(), ctx.summary_context()));
            for kw in ctx.search_keywords() {
                if !symbol.keywords.contains(&kw) {
                    symbol.keywords.push(kw);
                }
            }
        }
    }

    symbol.ecosystem_context = context_parts.join("; ");
}

// ── Extra import collection ────────────────────────────────────────────────

/// Collect extra imports from all providers, merging into `file_imports`.
/// Individual provider failures are caught so one bad provider doesn't block
/// others.
pub fn collect_extra_imports(
    file_imports: &mut FxHashMap<String, Vec<ExtraImport>>,
    providers: &[Box<dyn FrameworkProvider>],
) {
    for p in providers {
        let extras = match p.get_extra_imports() {
            Ok(e) => e,
            Err(_) => continue,
        };
        for (file, imports) in extras {
            let entry = file_imports.entry(file).or_default();
            for imp in imports {
                if !entry
                    .iter()
                    .any(|existing| existing.specifier == imp.specifier)
                {
                    entry.push(imp);
                }
            }
        }
    }
}

// ── Metadata collection ────────────────────────────────────────────────────

/// Collect metadata from all providers, with last-writer-wins for collisions.
/// Returns (merged metadata, list of collision warnings).
pub fn collect_metadata(
    providers: &[Box<dyn FrameworkProvider>],
) -> (FxHashMap<String, serde_json::Value>, Vec<String>) {
    let mut result: FxHashMap<String, serde_json::Value> = FxHashMap::default();
    let mut warnings: Vec<String> = Vec::new();

    for p in providers {
        let meta = p.get_metadata();
        for (key, value) in meta {
            if result.contains_key(&key) {
                warnings.push(format!("{} overwrites key \"{}\"", p.name(), key));
            }
            result.insert(key, value);
        }
    }

    (result, warnings)
}

// ── FrameworkRegistry ──────────────────────────────────────────────────────

/// Registry of framework provider factories.
pub struct FrameworkRegistry {
    factories: Vec<Box<dyn Fn() -> Box<dyn FrameworkProvider> + Send + Sync>>,
}

impl FrameworkRegistry {
    pub fn new() -> Self {
        Self {
            factories: Vec::new(),
        }
    }

    /// Register a provider factory.
    pub fn register<F>(&mut self, factory: F)
    where
        F: Fn() -> Box<dyn FrameworkProvider> + Send + Sync + 'static,
    {
        self.factories.push(Box::new(factory));
    }

    pub fn len(&self) -> usize {
        self.factories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    /// Discover which providers match `root`, load them, and return the active list.
    /// Providers whose `detect()` returns false are skipped.
    /// Providers whose `load()` fails are silently skipped.
    pub fn discover(&self, root: &Path) -> Vec<Box<dyn FrameworkProvider>> {
        let mut active: Vec<Box<dyn FrameworkProvider>> = Vec::new();
        for factory in &self.factories {
            let mut provider = factory();
            if provider.detect(root) {
                match provider.load(root) {
                    Ok(()) => active.push(provider),
                    Err(_) => { /* silently skip */ }
                }
            }
        }
        active
    }
}

impl Default for FrameworkRegistry {
    fn default() -> Self {
        Self::new()
    }
}
