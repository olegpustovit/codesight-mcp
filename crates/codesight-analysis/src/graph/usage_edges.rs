//! Symbol-usage file→file edges.
//!
//! `dependency_edges` is seeded from per-language import extraction
//! (`use`, `import`, `require`, …). That misses intra-crate access via
//! module paths — in Rust `mod foo; foo::Bar::thing()` crosses a file
//! boundary without ever writing `use foo::Bar`, so the importers graph
//! under-counts. Python sibling access through a package, Go sibling
//! access inside the same package, and C++ intra-unit references all
//! have similar shapes.
//!
//! This module derives a second adjacency — file `B` "uses" file `A`
//! when any symbol name defined in `A` appears as a word-boundary token
//! in `B`'s text, excluding `A == B`. It is deliberately language-agnostic:
//! anything the indexer has recorded as a symbol becomes a potential
//! target, and the scan operates on raw file content the same way
//! `find_references` does.
//!
//! The result is meant to be **unioned** with the stored import-based
//! adjacency, never to replace it. `use` edges remain the high-signal
//! path for languages that have explicit imports; the usage scan is a
//! structural fallback that catches intra-module access.

use std::sync::LazyLock;

use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};

use codesight_language_support::{is_code_extension, is_non_code_reference_path, is_vendored_path};
use codesight_storage::storage::IndexStore;

/// Short identifier names are near-guaranteed to collide with locals,
/// parameters, keywords, and English words in comments. Skip them to
/// keep the usage graph honest.
const MIN_SYMBOL_NAME_LEN: usize = 3;

/// Generic lifecycle / accessor / data-shape verbs — same name across
/// every codebase regardless of language. Per-language reserved words
/// (`fn`, `def`, `class`, …) come from
/// [`codesight_language_support::all_reserved_identifiers`]; this list
/// is the cross-language verb noise that no single language owns.
static GENERIC_VERB_STOPLIST: phf::Set<&'static str> = phf::phf_set! {
    "str", "int", "num", "val", "ref", "tag", "key", "row", "col", "idx",
    "err", "ctx", "buf", "out", "fmt", "doc", "end", "foo", "bar", "baz",
    "new", "init", "build", "make", "create", "start", "stop", "begin",
    "finish", "close", "open", "drop", "clone", "copy", "from", "into",
    "with", "get", "set", "add", "put", "run", "call", "apply", "exec",
    "map", "len", "count", "size", "empty", "clear", "push", "pop",
    "insert", "remove", "update", "delete", "find", "search", "lookup",
    "load", "store", "save", "read", "write", "fetch", "send", "recv",
    "parse", "format", "print", "log", "debug", "trace", "info", "warn",
    "check", "verify", "validate", "test", "filter", "transform",
    "encode", "decode", "serialize", "deserialize", "to_string", "display",
    "next", "prev", "last", "first", "peek", "poll", "wait", "notify",
    "hash", "equal", "compare", "cmp", "partial_cmp", "default", "value",
    "name", "path", "file", "line", "kind", "entry", "node",
    "data", "meta", "state", "status", "result", "error", "option",
};

static IDENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap());

fn build_identifier_stoplist() -> FxHashSet<String> {
    let mut set: FxHashSet<String> = GENERIC_VERB_STOPLIST.iter().map(|s| (*s).to_string()).collect();
    for kw in codesight_language_support::all_reserved_identifiers() {
        set.insert(kw.to_ascii_lowercase());
    }
    set
}

/// Symbol kinds that can legitimately be the *target* of a file→file
/// usage edge. Named types and top-level free functions collide rarely
/// by name, so matching their names across files is a high-signal
/// edge. Methods (`SavingsLedger.record`, `Db.load`) share names with
/// unrelated methods across the codebase; scanning for them produces
/// mostly noise.
///
/// We match on the leading identifier of the `kind` field — indexers
/// emit values like `struct`, `class`, `trait`, `interface`, `enum`,
/// `type`, `type_alias`, `function`, `fn`. Method-kinded symbols are
/// always excluded, regardless of their kind string.
fn is_edge_target_kind(kind: &str, name: &str) -> bool {
    let kind_lc = kind.to_ascii_lowercase();
    let is_named_type = matches!(
        kind_lc.as_str(),
        "class" | "struct" | "trait" | "interface" | "enum" | "type" | "type_alias"
    );
    if is_named_type {
        return true;
    }
    let is_callable = matches!(kind_lc.as_str(), "function" | "fn" | "procedure");
    if !is_callable {
        return false;
    }
    // Function-kinded symbols with a `.` in the name are methods (e.g.
    // `SavingsLedger.record`). Those share names across unrelated types
    // and produce noise. Top-level free functions are fine.
    !name.contains('.')
}

/// Return true if this path is plausible as the *source* of a symbol
/// reference edge. Vendored / fixture / asset paths and non-code file
/// types are excluded — a symbol name appearing in those files is
/// noise, not a real usage edge.
fn is_source_file_candidate(path: &str) -> bool {
    if is_vendored_path(path) || is_non_code_reference_path(path) {
        return false;
    }
    let ext = path.rsplit('.').next().unwrap_or("");
    if ext == path {
        return true; // no '.' in basename → no extension (Makefile, scripts)
    }
    is_code_extension(ext)
}

/// Build the **symbol-usage** adjacency: `src_file → [target_file, …]`
/// where `src_file` contains a word-boundary reference to some symbol
/// name that `target_file` defines.
///
/// Implementation walks each file's content once, extracts ASCII-identifier
/// tokens, and looks each one up in a `name → defining_files` index built
/// from the `symbols` table. Cost is O(Σ content_bytes) in the scan plus
/// O(#symbols) to build the index, which is well below the cold-dispatch
/// budget on the repos we target.
pub fn build_symbol_usage_adjacency(
    store: &IndexStore,
) -> anyhow::Result<FxHashMap<String, Vec<String>>> {
    let symbols = store.all_symbols_with_source()?;
    let contents = store.all_file_contents().unwrap_or_default();
    let stoplist = build_identifier_stoplist();

    let mut by_name: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    for sym in &symbols {
        // Only named types and top-level free functions produce
        // high-signal edges. Methods collide on name across types
        // (every type has a `record`, `load`, `store`); treating each
        // as an edge target would make the graph mostly noise.
        if !is_edge_target_kind(&sym.kind, &sym.name) {
            continue;
        }
        // Extract the *terminal* identifier — for methods that slip
        // through (we've already filtered them, but defensive) or for
        // qualified function names. `SavingsLedger.record` → `record`.
        let terminal_name = sym.name.rsplit('.').next().unwrap_or(&sym.name);
        if terminal_name.len() < MIN_SYMBOL_NAME_LEN {
            continue;
        }
        if stoplist.contains(&terminal_name.to_ascii_lowercase()) {
            continue;
        }
        // Symbols "defined" in a non-source file (markdown heading, JSON
        // vocab entry, SCM query capture) aren't real targets for a
        // usage edge — excluding them matches find_references behavior
        // and keeps the graph honest.
        if !is_source_file_candidate(&sym.file) {
            continue;
        }
        by_name
            .entry(terminal_name.to_string())
            .or_default()
            .insert(sym.file.clone());
    }

    let mut adj: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    for (file_path, content) in &contents {
        // A roadmap, tokenizer dump, or tree-sitter query file can't
        // "import" a symbol. Skip the whole scan for these — also the
        // cheapest filter we can apply before the per-token loop.
        if !is_source_file_candidate(file_path) {
            continue;
        }
        for m in IDENT_RE.find_iter(content) {
            let Some(defining_files) = by_name.get(m.as_str()) else {
                continue;
            };
            for def_file in defining_files {
                if def_file == file_path {
                    continue;
                }
                adj.entry(file_path.clone())
                    .or_default()
                    .insert(def_file.clone());
            }
        }
    }

    Ok(adj
        .into_iter()
        .map(|(src, tgts)| {
            let mut v: Vec<String> = tgts.into_iter().collect();
            v.sort();
            (src, v)
        })
        .collect())
}

/// Union two file→file adjacency maps. Targets per key are deduped and
/// sorted so downstream consumers see stable output.
///
/// Primary use: combine the stored `dependency_graph()` (explicit-import
/// edges) with `build_symbol_usage_adjacency` (symbol-reference edges).
pub fn union_adjacency(
    primary: &FxHashMap<String, Vec<String>>,
    secondary: &FxHashMap<String, Vec<String>>,
) -> FxHashMap<String, Vec<String>> {
    let mut merged: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    for (src, targets) in primary {
        merged
            .entry(src.clone())
            .or_default()
            .extend(targets.iter().cloned());
    }
    for (src, targets) in secondary {
        merged
            .entry(src.clone())
            .or_default()
            .extend(targets.iter().cloned());
    }
    merged
        .into_iter()
        .map(|(src, tgts)| {
            let mut v: Vec<String> = tgts.into_iter().collect();
            v.sort();
            (src, v)
        })
        .collect()
}

/// Return the stored import-graph adjacency unioned with the
/// symbol-usage adjacency. Files B that reach a symbol in A through a
/// module path (Rust `mod … ; a::b::X`, Python package sibling, Go
/// same-package access) show up alongside explicit-import edges.
pub fn adjacency_with_usage_edges(
    store: &IndexStore,
) -> anyhow::Result<FxHashMap<String, Vec<String>>> {
    let primary = store.dependency_graph()?;
    let secondary = build_symbol_usage_adjacency(store)?;
    Ok(union_adjacency(&primary, &secondary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_re_yields_only_word_tokens() {
        let got: Vec<&str> = IDENT_RE
            .find_iter("fn foo(bar: Baz) -> i32 { 42 + quux_1 }")
            .map(|m| m.as_str())
            .collect();
        assert_eq!(got, vec!["fn", "foo", "bar", "Baz", "i32", "quux_1"]);
    }

    #[test]
    fn is_source_file_candidate_rejects_prose_data_and_vendored() {
        // Prose / docs
        assert!(!is_source_file_candidate("docs/ROADMAP.md"));
        assert!(!is_source_file_candidate("README.rst"));
        // Data / config
        assert!(!is_source_file_candidate(
            "crates/codesight-retrieval/assets/embedder/potion-base-8M/tokenizer.json"
        ));
        assert!(!is_source_file_candidate("Cargo.toml"));
        // Tree-sitter query
        assert!(!is_source_file_candidate(
            "crates/codesight-language-support/src/languages/rust/queries/tags.scm"
        ));
        // Fixture path (vendored by segment, even though .cs is code)
        assert!(!is_source_file_candidate(
            "crates/codesight-language-support/tests/fixtures/code_reference/csharp/sample.cs"
        ));
        // Real source files pass
        assert!(is_source_file_candidate(
            "crates/codesight-engine/src/service/savings/ledger.rs"
        ));
        assert!(is_source_file_candidate("src/main.py"));
        assert!(is_source_file_candidate("cmd/server/main.go"));
        // Files without extension pass (Makefile, shell scripts, etc.)
        assert!(is_source_file_candidate("Makefile"));
    }

    #[test]
    fn is_edge_target_kind_accepts_types_and_top_level_functions() {
        // Named types — always high-signal
        assert!(is_edge_target_kind("struct", "SavingsLedger"));
        assert!(is_edge_target_kind("class", "IndexStore"));
        assert!(is_edge_target_kind("trait", "Language"));
        assert!(is_edge_target_kind("interface", "Reader"));
        assert!(is_edge_target_kind("enum", "FileClass"));
        assert!(is_edge_target_kind("type", "SymbolId"));
        // Top-level free functions
        assert!(is_edge_target_kind("function", "adjacency_with_usage_edges"));
        assert!(is_edge_target_kind("fn", "build_symbol_usage_adjacency"));
    }

    #[test]
    fn is_edge_target_kind_rejects_methods_and_other_kinds() {
        // Methods (name contains `.`) — name collision is rampant
        assert!(!is_edge_target_kind("function", "SavingsLedger.record"));
        assert!(!is_edge_target_kind("function", "IndexStore.load"));
        assert!(!is_edge_target_kind("fn", "Db.store"));
        // Fields / locals / constants / modules — not callable names
        assert!(!is_edge_target_kind("field", "total_tokens_saved"));
        assert!(!is_edge_target_kind("constant", "DEFAULT_BUDGET"));
        assert!(!is_edge_target_kind("module", "savings"));
        assert!(!is_edge_target_kind("variable", "ledger"));
    }

    #[test]
    fn is_source_file_candidate_is_case_insensitive() {
        assert!(!is_source_file_candidate("NOTES.MD"));
        assert!(!is_source_file_candidate("data.JSON"));
    }

    #[test]
    fn union_adjacency_dedupes_and_sorts() {
        let mut primary: FxHashMap<String, Vec<String>> = FxHashMap::default();
        primary.insert("a".into(), vec!["b".into(), "c".into()]);
        let mut secondary: FxHashMap<String, Vec<String>> = FxHashMap::default();
        secondary.insert("a".into(), vec!["c".into(), "d".into()]);
        secondary.insert("x".into(), vec!["y".into()]);

        let merged = union_adjacency(&primary, &secondary);
        assert_eq!(merged["a"], vec!["b", "c", "d"]);
        assert_eq!(merged["x"], vec!["y"]);
    }
}
