//! Phase-0 regression harness from FEEDBACK.md: asserts that the
//! reachability graph follows workspace-crate `use`, `pub use` re-exports,
//! and binary-target live roots. Without these, the analysis tools fire
//! confident-wrong dead-code verdicts for any file reached through a
//! cross-crate re-export chain.
//!
//! Two fixtures:
//! - synthetic workspace (binary + lib crate, pub-use depth-3 chain)
//! - real repo root (guards against drift in this codebase specifically)

use std::fs;
use std::path::Path;

use dowser_analysis::dead_code::dead_code_payload;
use dowser_mcp::test_support::{index_code_fixture, open_code_store, write_files};
use dowser_storage::storage::IndexStore;
use serde_json::{Value, json};
use tempfile::tempdir;

fn dead_files(store: &IndexStore) -> Vec<String> {
    let payload = dead_code_payload(store, "file", 0.5, true, &[], None, false, false).unwrap();
    payload["dead_files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d["file"].as_str().map(str::to_string))
        .collect()
}

fn build_synthetic_workspace(root: &Path) {
    // Workspace root.
    fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/lib-a", "crates/bin-a"]
resolver = "2"
"#,
    )
    .unwrap();

    // Library crate with a pub-use re-export chain of depth 3.
    write_files(
        root,
        &[
            (
                "crates/lib-a/Cargo.toml",
                r#"[package]
name = "lib-a"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "crates/lib-a/src/lib.rs",
                "pub mod facade;\npub use facade::api;\n",
            ),
            (
                "crates/lib-a/src/facade.rs",
                "pub mod inner;\npub use inner::api;\n",
            ),
            (
                "crates/lib-a/src/inner.rs",
                "pub fn api() -> &'static str { \"hi\" }\n",
            ),
            // A sibling file only reached from inner.rs — verifies transitive
            // reachability: bin → lib.rs → facade.rs → inner.rs → deep.rs
            ("crates/lib-a/src/deep.rs", "pub fn deep() -> u32 { 42 }\n"),
        ],
    )
    .unwrap();
    // Append a `use crate::deep;` edge to inner.rs so the full chain
    // terminates at deep.rs.
    fs::write(
        root.join("crates/lib-a/src/inner.rs"),
        "use crate::deep;\npub fn api() -> u32 { deep::deep() }\n",
    )
    .unwrap();

    // Binary crate that imports the library via `use lib_a::api`.
    write_files(
        root,
        &[
            (
                "crates/bin-a/Cargo.toml",
                r#"[package]
name = "bin-a"
version = "0.1.0"
edition = "2024"

[dependencies]
lib-a = { path = "../lib-a" }
"#,
            ),
            (
                "crates/bin-a/src/main.rs",
                "use lib_a::api;\nfn main() { println!(\"{}\", api()); }\n",
            ),
        ],
    )
    .unwrap();
}

#[test]
fn dead_code_respects_workspace_crate_use_and_pub_use_chain() {
    let ws = tempdir().unwrap();
    build_synthetic_workspace(ws.path());

    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();

    let dead = dead_files(&store);

    // Every file in the lib-a pub-use chain must be reachable from bin-a's main.
    for expected in [
        "crates/lib-a/src/lib.rs",
        "crates/lib-a/src/facade.rs",
        "crates/lib-a/src/inner.rs",
        "crates/lib-a/src/deep.rs",
    ] {
        assert!(
            !dead.iter().any(|f| f == expected),
            "regression: {expected} marked dead — dead_files={dead:?}",
        );
    }
}

#[test]
fn binary_main_rs_is_always_a_live_root() {
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    write_files(
        ws.path(),
        &[("src/main.rs", "fn main() { println!(\"hi\"); }\n")],
    )
    .unwrap();

    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();

    let dead = dead_files(&store);
    assert!(
        !dead.iter().any(|f| f == "src/main.rs"),
        "regression: src/main.rs marked dead — dead_files={dead:?}",
    );
}

#[test]
fn src_bin_targets_are_live_roots() {
    // `src/bin/*.rs` is cargo's default auto-discovery for extra binaries.
    // They must count as live roots even without an explicit `[[bin]]` entry.
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        r#"[package]
name = "demo"
version = "0.1.0"
edition = "2024"
"#,
    )
    .unwrap();
    write_files(
        ws.path(),
        &[
            ("src/lib.rs", "pub fn lib_only() {}\n"),
            ("src/bin/tool.rs", "fn main() { println!(\"tool\"); }\n"),
        ],
    )
    .unwrap();

    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();

    let dead = dead_files(&store);
    assert!(
        !dead.iter().any(|f| f == "src/bin/tool.rs"),
        "regression: src/bin/tool.rs marked dead — dead_files={dead:?}",
    );
}

#[test]
fn importers_resolve_cross_crate_use() {
    // After Phase 1, the dependency graph must include an edge from
    // bin-a/src/main.rs → lib-a/src/lib.rs (via `use lib_a::api`).
    let ws = tempdir().unwrap();
    build_synthetic_workspace(ws.path());
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();

    let graph = store.dependency_graph().unwrap();
    let targets = graph
        .get("crates/bin-a/src/main.rs")
        .cloned()
        .unwrap_or_default();
    assert!(
        targets.iter().any(|t| t == "crates/lib-a/src/lib.rs"),
        "regression: bin-a/main.rs has no edge to lib-a/lib.rs — targets={targets:?}",
    );
}

#[test]
fn dead_code_reaches_files_declared_via_mod_only() {
    // P0.1 regression: a submodule introduced only via `mod foo;` in
    // the parent's `mod.rs` (no `use` elsewhere) must still be reachable.
    // Without module-tree edges, every deep submodule appears dead.
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    write_files(
        ws.path(),
        &[
            ("src/lib.rs", "pub mod service;\n"),
            ("src/service/mod.rs", "pub mod inner;\n"),
            ("src/service/inner.rs", "pub fn deep() -> u32 { 1 }\n"),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    let dead = dead_files(&store);
    for expected in ["src/service/mod.rs", "src/service/inner.rs"] {
        assert!(
            !dead.iter().any(|f| f == expected),
            "regression: {expected} marked dead — module-tree edges broken; dead={dead:?}",
        );
    }
}

#[test]
fn importers_see_module_tree_parent() {
    // P0.1: `graph=importers` on a submodule must include its parent's mod.rs
    // even when nothing else `use`s it.
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    write_files(
        ws.path(),
        &[
            ("src/lib.rs", "pub mod service;\n"),
            ("src/service/mod.rs", "pub mod inner;\n"),
            ("src/service/inner.rs", "pub fn deep() {}\n"),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    let graph = store.dependency_graph().unwrap();
    let importers: Vec<String> = graph
        .iter()
        .filter(|(_, targets)| targets.iter().any(|t| t == "src/service/inner.rs"))
        .map(|(src, _)| src.clone())
        .collect();
    assert!(
        importers.iter().any(|i| i == "src/service/mod.rs"),
        "regression: src/service/inner.rs has no mod-tree parent — importers={importers:?}",
    );
}

#[test]
fn impl_drop_on_otherwise_unreferenced_struct_stays_live() {
    // Phase 2: a struct that exists only to satisfy `impl Drop for X` has
    // no name-reference edge. Reachability via `foreign_trait_impl_files`
    // must keep its enclosing file live.
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n",
    )
    .unwrap();
    write_files(
        ws.path(),
        &[
            ("src/main.rs", "fn main() {}\n"),
            (
                "src/guard.rs",
                "pub struct Guard;\nimpl Drop for Guard { fn drop(&mut self) {} }\n",
            ),
        ],
    )
    .unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();
    let dead = dead_files(&store);
    assert!(
        !dead.iter().any(|f| f == "src/guard.rs"),
        "regression: impl Drop file marked dead — dead={dead:?}",
    );
}

/// Envelope smoke: the first three assertions all fail loudly when the
/// `dead_code` payload shape changes. Bundled into one synthetic test so we
/// notice contract drift without paying for another full reindex.
#[test]
fn dead_code_payload_has_expected_keys() {
    let ws = tempdir().unwrap();
    fs::write(
        ws.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    write_files(ws.path(), &[("src/main.rs", "fn main() {}\n")]).unwrap();
    let (config, sid, _) = index_code_fixture(&ws.path().join("idx"), ws.path()).unwrap();
    let store = open_code_store(&config, &sid).unwrap();

    let payload: Value =
        dead_code_payload(&store, "file", 0.5, true, &[], None, false, false).unwrap();
    for key in [
        "analysis",
        "dead_files",
        "dead_file_count",
        "live_root_count",
    ] {
        assert!(payload.get(key).is_some(), "missing key {key} in {payload}");
    }
    assert_eq!(payload["analysis"], json!("dead_code"));
}
