//! Structural reachability edges — produced per-file by [`Language`]
//! implementations at ingest time, persisted by storage, consumed by
//! `dead_code` analysis.
//!
//! Replaces the in-process file-scan augmentation passes that used to live
//! in `symslice-analysis::dead_code::reachability`. Moving the extraction
//! into the language trait means cross-language coverage is automatic and
//! the dead-code call drops from ~400ms (5 file-scan passes) to a single
//! SQL read.
//!
//! [`Language`]: symslice_language_support::Language

use serde::{Deserialize, Serialize};

/// Kind of structural reachability edge.
///
/// Each variant has a stable string form (`as_str` / `parse_kind`) used as
/// the SQLite `edge_kind` column value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// `pub use foo::Bar` — re-export. Source file points at definer.
    ReExport,
    /// `self::child::fn()` / `crate::a::b::fn()` path reference.
    PathReference,
    /// `=> self.method(...)` enum-match-arm dispatch (and similar).
    MatchDispatch,
    /// `mod child;` declaration in a parent file.
    ModuleDecl,
    /// Any import statement (`use`, `import`, `require`, `from … import`).
    ImportStatement,
    /// Generic / dyn Trait usage — keeps a type-only reference alive.
    TypeRef,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::ReExport => "reexport",
            EdgeKind::PathReference => "path_ref",
            EdgeKind::MatchDispatch => "match_dispatch",
            EdgeKind::ModuleDecl => "module_decl",
            EdgeKind::ImportStatement => "import",
            EdgeKind::TypeRef => "type_ref",
        }
    }

    pub fn parse_kind(s: &str) -> Option<Self> {
        Some(match s {
            "reexport" => EdgeKind::ReExport,
            "path_ref" => EdgeKind::PathReference,
            "match_dispatch" => EdgeKind::MatchDispatch,
            "module_decl" => EdgeKind::ModuleDecl,
            "import" => EdgeKind::ImportStatement,
            "type_ref" => EdgeKind::TypeRef,
            _ => return None,
        })
    }
}

/// A single structural edge a language extracted from a file.
///
/// At least one of `target_file` / `target_symbol` should be `Some` —
/// edges with neither are pruned at the storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StructuralEdge {
    pub source_file: String,
    /// Repo-relative path of the file this edge targets, when the
    /// language was able to resolve it.
    pub target_file: Option<String>,
    /// Bare name of the symbol this edge targets (no path prefix). Used
    /// for symbol-level dead detection.
    pub target_symbol: Option<String>,
    pub edge_kind: EdgeKind,
}

impl StructuralEdge {
    pub fn new_symbol(
        source_file: impl Into<String>,
        symbol: impl Into<String>,
        kind: EdgeKind,
    ) -> Self {
        Self {
            source_file: source_file.into(),
            target_file: None,
            target_symbol: Some(symbol.into()),
            edge_kind: kind,
        }
    }

    pub fn new_file(
        source_file: impl Into<String>,
        target_file: impl Into<String>,
        kind: EdgeKind,
    ) -> Self {
        Self {
            source_file: source_file.into(),
            target_file: Some(target_file.into()),
            target_symbol: None,
            edge_kind: kind,
        }
    }

    pub fn is_useful(&self) -> bool {
        self.target_file.is_some() || self.target_symbol.is_some()
    }
}
