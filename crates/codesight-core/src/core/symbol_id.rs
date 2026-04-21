//! Typed symbol identifier.
//!
//! A `symbol_id` is the stable handle every tool references — written to
//! SQLite, returned in MCP responses, passed back in subsequent lookups.
//! The string grammar is:
//!
//! ```text
//!   [repo "::"] file "::" path_parts "#" kind
//!   path_parts := scope ("." scope)*
//! ```
//!
//! Examples:
//! - `"src/foo.rs::do_work#function"` — file + leaf + kind
//! - `"src/foo.rs::tests.test_do_work#function"` — nested scope `tests`
//! - `"app/Models/User.php::User.posts#method"` — parent class + method
//! - `"repo::src/lib.py::helper#function"` — repo-qualified
//!
//! `SymbolIdRef` is a zero-cost borrowed view; every accessor is a single
//! `rsplit` / `split` on the underlying byte slice (no allocation). Use it
//! in trait methods, hot loops, and any parsing-adjacent code. `SymbolId`
//! is the owned variant for storage-row boundaries where a `String` is the
//! natural shape.

use std::fmt;

/// Separator between the file/repo scope and the symbol segment.
const SCOPE_SEP: &str = "::";
/// Separator between nested scopes inside the symbol segment.
const PATH_SEP: char = '.';
/// Separator between the symbol path and the kind suffix.
const KIND_SEP: char = '#';

/// Zero-cost borrowed view over a stable `symbol_id`. Every accessor runs a
/// single byte-range lookup on the backing slice; nothing is copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolIdRef<'a> {
    raw: &'a str,
}

impl<'a> SymbolIdRef<'a> {
    pub const fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    pub const fn as_str(&self) -> &'a str {
        self.raw
    }

    /// Everything before the final `::`. Empty when the id is just a
    /// bare `name#kind` segment (rare, but tolerated).
    pub fn file_scope(&self) -> &'a str {
        match self.raw.rsplit_once(SCOPE_SEP) {
            Some((scope, _)) => scope,
            None => "",
        }
    }

    /// Everything after the final `::` — i.e. `path.leaf#kind`. When no
    /// `::` is present this is the whole string.
    pub fn symbol_segment(&self) -> &'a str {
        match self.raw.rsplit_once(SCOPE_SEP) {
            Some((_, segment)) => segment,
            None => self.raw,
        }
    }

    /// The kind suffix (`class`, `function`, `method`, …) or `None` when
    /// the id carries no `#kind` marker.
    pub fn kind(&self) -> Option<&'a str> {
        self.symbol_segment()
            .rsplit_once(KIND_SEP)
            .map(|(_, kind)| kind)
            .filter(|k| !k.is_empty())
    }

    /// The symbol-path portion (`tests.test_do_work`, `User.posts`) with
    /// the `#kind` suffix stripped.
    pub fn symbol_path(&self) -> &'a str {
        let segment = self.symbol_segment();
        match segment.rsplit_once(KIND_SEP) {
            Some((path, _)) => path,
            None => segment,
        }
    }

    /// Terminal segment of the symbol path — the symbol's own name.
    pub fn leaf_name(&self) -> &'a str {
        let path = self.symbol_path();
        match path.rsplit_once(PATH_SEP) {
            Some((_, leaf)) => leaf,
            None => path,
        }
    }

    /// The symbol path minus its leaf — useful for parent-scope checks.
    /// Empty when the symbol has no parent scope.
    pub fn parent_path(&self) -> &'a str {
        let path = self.symbol_path();
        match path.rsplit_once(PATH_SEP) {
            Some((parent, _)) => parent,
            None => "",
        }
    }

    /// Iterate over the `.`-separated parent scopes in source order.
    pub fn scopes(&self) -> impl Iterator<Item = &'a str> {
        let parent = self.parent_path();
        parent.split(PATH_SEP).filter(|s| !s.is_empty())
    }

    /// True when `name` appears as one of the parent scopes. Used by
    /// per-language `is_test_symbol_id` overrides (e.g. Rust checks
    /// `has_scope("tests")`).
    pub fn has_scope(&self, name: &str) -> bool {
        self.scopes().any(|s| s == name)
    }
}

impl<'a> fmt::Display for SymbolIdRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.raw)
    }
}

/// Owned stable symbol identifier. Storage rows and MCP responses hand back
/// `String`s; wrap them in a `SymbolId` to keep the typed distinction and
/// avoid accidental parsing at call sites.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolId(String);

impl SymbolId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Construct from components: `<file>::<path>#<kind>`. When `path`
    /// contains a parent scope use the dotted form (`"tests.my_test"`).
    pub fn from_parts(file: &str, path: &str, kind: &str) -> Self {
        Self(format!("{file}{SCOPE_SEP}{path}{KIND_SEP}{kind}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn as_ref(&self) -> SymbolIdRef<'_> {
        SymbolIdRef::new(&self.0)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SymbolId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SymbolId {
    fn from(raw: String) -> Self {
        Self(raw)
    }
}

impl From<&str> for SymbolId {
    fn from(raw: &str) -> Self {
        Self(raw.to_string())
    }
}

impl<'a> From<&'a SymbolId> for SymbolIdRef<'a> {
    fn from(owned: &'a SymbolId) -> Self {
        owned.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sref(s: &str) -> SymbolIdRef<'_> {
        SymbolIdRef::new(s)
    }

    #[test]
    fn plain_file_symbol() {
        let id = sref("src/foo.rs::do_work#function");
        assert_eq!(id.file_scope(), "src/foo.rs");
        assert_eq!(id.symbol_segment(), "do_work#function");
        assert_eq!(id.symbol_path(), "do_work");
        assert_eq!(id.leaf_name(), "do_work");
        assert_eq!(id.parent_path(), "");
        assert_eq!(id.kind(), Some("function"));
        assert_eq!(id.scopes().count(), 0);
    }

    #[test]
    fn rust_inline_test_scope() {
        let id = sref("src/foo.rs::tests.test_do_work#function");
        assert_eq!(id.symbol_path(), "tests.test_do_work");
        assert_eq!(id.leaf_name(), "test_do_work");
        assert_eq!(id.parent_path(), "tests");
        assert!(id.has_scope("tests"));
        assert!(!id.has_scope("test_do_work"));
    }

    #[test]
    fn php_method_inside_class() {
        let id = sref("app/Models/User.php::User.posts#method");
        assert_eq!(id.file_scope(), "app/Models/User.php");
        assert_eq!(id.leaf_name(), "posts");
        assert_eq!(id.parent_path(), "User");
        assert_eq!(id.kind(), Some("method"));
    }

    #[test]
    fn repo_qualified_id_keeps_repo_in_file_scope() {
        let id = sref("repo::src/lib.py::helper#function");
        assert_eq!(id.file_scope(), "repo::src/lib.py");
        assert_eq!(id.leaf_name(), "helper");
        assert_eq!(id.kind(), Some("function"));
    }

    #[test]
    fn missing_kind_is_none() {
        let id = sref("src/foo.rs::do_work");
        assert_eq!(id.leaf_name(), "do_work");
        assert_eq!(id.kind(), None);
    }

    #[test]
    fn bare_name_without_scope() {
        let id = sref("Usage");
        assert_eq!(id.file_scope(), "");
        assert_eq!(id.symbol_segment(), "Usage");
        assert_eq!(id.leaf_name(), "Usage");
        assert_eq!(id.kind(), None);
    }

    #[test]
    fn from_parts_round_trips() {
        let id = SymbolId::from_parts("src/foo.rs", "tests.test_do_work", "function");
        assert_eq!(id.as_str(), "src/foo.rs::tests.test_do_work#function");
        assert_eq!(id.as_ref().leaf_name(), "test_do_work");
        assert!(id.as_ref().has_scope("tests"));
    }
}
