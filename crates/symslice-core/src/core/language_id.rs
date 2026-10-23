//! Typed language identifier (LANGUAGES.md §7.1.1).
//!
//! `LanguageId` is a `Copy` newtype over `&'static str`. It lives in
//! `symslice-core` (L0) so pipeline types (`StoredSymbol`,
//! `IndexedFileUpdate`, `FileFingerprint`) can carry typed language
//! identity without a layer violation. The language constants and all
//! detection logic still live in `symslice-language-support`, which
//! re-exports the type.
//!
//! Runtime string inputs (e.g. a language column read back from SQLite)
//! are resolved through `LanguageId::intern`, which leaks an `&'static
//! str` on first encounter. Leaks are bounded — the language set is
//! ~50 entries — and the fast path for known constants bypasses
//! allocation entirely.

use rustc_hash::FxHashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct LanguageId(&'static str);

impl LanguageId {
    pub const fn new(s: &'static str) -> Self {
        Self(s)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }

    /// Resolve a runtime string to a typed `LanguageId`.
    ///
    /// Known constants (`RUST`, `PYTHON`, …) short-circuit through
    /// `CANONICAL_IDS` with zero allocation. Unknown strings are
    /// leaked into `&'static str` on first encounter and cached for
    /// subsequent calls. This keeps `LanguageId` `Copy` while still
    /// allowing storage round-trips to produce a typed value.
    pub fn intern(s: &str) -> Self {
        if let Some(canonical) = canonical_lookup(s) {
            return Self(canonical);
        }
        intern_runtime(s)
    }

    /// Same as `intern` but returns `UNKNOWN_LANGUAGE` when the string
    /// is not part of the canonical set. Prefer this when you never
    /// want to pay the interning-leak cost.
    pub fn canonical_or_unknown(s: &str) -> Self {
        canonical_lookup(s).map_or(UNKNOWN_LANGUAGE, Self)
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl serde::Serialize for LanguageId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> serde::Deserialize<'de> for LanguageId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &str = <&str>::deserialize(deserializer)?;
        Ok(Self::intern(s))
    }
}

// Perfect-hash lookup table for canonical language names. Compile-time
// generated; O(1) lookup with zero cyclomatic complexity in the caller.
static CANONICAL_IDS: phf::Map<&'static str, &'static str> = phf::phf_map! {
    "unknown" => UNKNOWN_LANGUAGE.0,
    "rust" => RUST.0,
    "typescript" => TYPESCRIPT.0,
    "tsx" => TSX.0,
    "javascript" => JAVASCRIPT.0,
    "jsx" => JSX.0,
    "python" => PYTHON.0,
    "go" => GO.0,
    "java" => JAVA.0,
    "ruby" => RUBY.0,
    "php" => PHP.0,
    "csharp" => CSHARP.0,
    "cpp" => CPP.0,
    "c" => C.0,
    "swift" => SWIFT.0,
    "kotlin" => KOTLIN.0,
    "dart" => DART.0,
    "bash" => BASH.0,
    "sql" => SQL.0,
    "yaml" => YAML.0,
    "json" => JSON.0,
    "xml" => XML.0,
    "css" => CSS.0,
    "scss" => SCSS.0,
    "less" => LESS.0,
    "sass" => SASS.0,
    "lua" => LUA.0,
    "luau" => LUAU.0,
    "nix" => NIX.0,
    "perl" => PERL.0,
    "asm" => ASM.0,
    "blade" => BLADE.0,
    "razor" => RAZOR.0,
    "ejs" => EJS.0,
    "verse" => VERSE.0,
    "elixir" => ELIXIR.0,
    "vue" => VUE.0,
    "al" => AL.0,
    "styl" => STYL.0,
    "erlang" => ERLANG.0,
    "fortran" => FORTRAN.0,
    "gleam" => GLEAM.0,
    "ocaml" => OCAML.0,
    "ocaml-interface" => OCAML_INTERFACE.0,
    "markdown" => MARKDOWN.0,
    "toml" => TOML.0,
    "tree-sitter-query" => TREE_SITTER_QUERY.0,
    "openapi" => OPENAPI.0,
    "ansible" => ANSIBLE.0,
};

fn canonical_lookup(s: &str) -> Option<&'static str> {
    CANONICAL_IDS.get(s).copied()
}

fn intern_runtime(s: &str) -> LanguageId {
    static INTERN: OnceLock<Mutex<FxHashMap<Box<str>, &'static str>>> = OnceLock::new();
    let table = INTERN.get_or_init(|| Mutex::new(FxHashMap::default()));
    let mut guard = table.lock().expect("LanguageId intern mutex poisoned");
    if let Some(&leaked) = guard.get(s) {
        return LanguageId(leaked);
    }
    let owned: Box<str> = s.into();
    let leaked: &'static str = Box::leak(owned.clone());
    guard.insert(owned, leaked);
    LanguageId(leaked)
}

// ── Builtin language identifiers ────────────────────────────────────────
//
// These are the canonical language strings used everywhere in the
// workspace. They live at L0 so `StoredSymbol.language: LanguageId`
// can reference them without depending on the language-support crate.

pub const UNKNOWN_LANGUAGE: LanguageId = LanguageId::new("unknown");
pub const RUST: LanguageId = LanguageId::new("rust");
pub const TYPESCRIPT: LanguageId = LanguageId::new("typescript");
pub const TSX: LanguageId = LanguageId::new("tsx");
pub const JAVASCRIPT: LanguageId = LanguageId::new("javascript");
pub const JSX: LanguageId = LanguageId::new("jsx");
pub const PYTHON: LanguageId = LanguageId::new("python");
pub const GO: LanguageId = LanguageId::new("go");
pub const JAVA: LanguageId = LanguageId::new("java");
pub const RUBY: LanguageId = LanguageId::new("ruby");
pub const PHP: LanguageId = LanguageId::new("php");
pub const CSHARP: LanguageId = LanguageId::new("csharp");
pub const CPP: LanguageId = LanguageId::new("cpp");
pub const C: LanguageId = LanguageId::new("c");
pub const SWIFT: LanguageId = LanguageId::new("swift");
pub const KOTLIN: LanguageId = LanguageId::new("kotlin");
pub const DART: LanguageId = LanguageId::new("dart");
pub const BASH: LanguageId = LanguageId::new("bash");
pub const SQL: LanguageId = LanguageId::new("sql");
pub const YAML: LanguageId = LanguageId::new("yaml");
pub const JSON: LanguageId = LanguageId::new("json");
pub const XML: LanguageId = LanguageId::new("xml");
pub const CSS: LanguageId = LanguageId::new("css");
pub const SCSS: LanguageId = LanguageId::new("scss");
pub const LESS: LanguageId = LanguageId::new("less");
pub const SASS: LanguageId = LanguageId::new("sass");
pub const LUA: LanguageId = LanguageId::new("lua");
pub const LUAU: LanguageId = LanguageId::new("luau");
pub const NIX: LanguageId = LanguageId::new("nix");
pub const PERL: LanguageId = LanguageId::new("perl");
pub const ASM: LanguageId = LanguageId::new("asm");
pub const BLADE: LanguageId = LanguageId::new("blade");
pub const RAZOR: LanguageId = LanguageId::new("razor");
pub const EJS: LanguageId = LanguageId::new("ejs");
pub const VERSE: LanguageId = LanguageId::new("verse");
pub const ELIXIR: LanguageId = LanguageId::new("elixir");
pub const VUE: LanguageId = LanguageId::new("vue");
pub const AL: LanguageId = LanguageId::new("al");
pub const STYL: LanguageId = LanguageId::new("styl");
pub const ERLANG: LanguageId = LanguageId::new("erlang");
pub const FORTRAN: LanguageId = LanguageId::new("fortran");
pub const GLEAM: LanguageId = LanguageId::new("gleam");
pub const OCAML: LanguageId = LanguageId::new("ocaml");
pub const OCAML_INTERFACE: LanguageId = LanguageId::new("ocaml-interface");
pub const MARKDOWN: LanguageId = LanguageId::new("markdown");
pub const TOML: LanguageId = LanguageId::new("toml");
pub const TREE_SITTER_QUERY: LanguageId = LanguageId::new("tree-sitter-query");
pub const OPENAPI: LanguageId = LanguageId::new("openapi");
pub const ANSIBLE: LanguageId = LanguageId::new("ansible");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_as_str_roundtrip() {
        let id = LanguageId::new("rust");
        assert_eq!(id.as_str(), "rust");
        assert_eq!(id, RUST);
    }

    #[test]
    fn intern_canonical_returns_same_static_pointer() {
        let a = LanguageId::intern("rust");
        let b = RUST;
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.as_str().as_ptr(), b.as_str().as_ptr()));
    }

    #[test]
    fn intern_runtime_string_stabilises() {
        let a = LanguageId::intern("made-up-lang");
        let b = LanguageId::intern("made-up-lang");
        assert_eq!(a, b);
        assert!(std::ptr::eq(a.as_str().as_ptr(), b.as_str().as_ptr()));
    }

    #[test]
    fn canonical_or_unknown_falls_back() {
        assert_eq!(LanguageId::canonical_or_unknown("rust"), RUST);
        assert_eq!(
            LanguageId::canonical_or_unknown("never-heard-of-it"),
            UNKNOWN_LANGUAGE
        );
    }

    #[test]
    fn display_and_debug_shape() {
        assert_eq!(format!("{}", RUST), "rust");
        assert!(format!("{:?}", RUST).contains("rust"));
    }
}
