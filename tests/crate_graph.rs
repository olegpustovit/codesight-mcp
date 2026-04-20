//! CI gate: assert the workspace crate dependency invariants still hold.
//!
//! Encodes MEGAPLAN §3.4: L0 primitives → L0.5 backend trait → L1 engine →
//! L2 protocol → L3 harness (future). Each layer may only depend on layers
//! below it. Concrete backend implementations never leak through the engine.

#[test]
fn workspace_crate_dependency_invariants_hold() {
    let metadata = cargo_metadata::MetadataCommand::new().exec().unwrap();
    let workspace_members: Vec<&cargo_metadata::Package> = metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
        .collect();

    let names: Vec<&str> = workspace_members.iter().map(|p| p.name.as_str()).collect();
    for expected in [
        "codesight-mcp",
        "codesight-analysis",
        "codesight-backend",
        "codesight-core",
        "codesight-engine",
        "codesight-extractors",
        "codesight-imports",
        "codesight-language-support",
        "codesight-retrieval",
        "codesight-storage",
        "mcp-rpc",
    ] {
        assert!(
            names.contains(&expected),
            "missing workspace crate: {expected}"
        );
    }

    // Only normal (non-dev, non-build) deps count for architectural layering.
    // Dev-deps legitimately wire test-only backends (e.g. engine pulls in the
    // tree-sitter backend in `[dev-dependencies]` to exercise ingest paths).
    let ws_deps = |pkg_name: &str| -> Vec<String> {
        let pkg = workspace_members
            .iter()
            .find(|p| p.name == pkg_name)
            .unwrap();
        let mut deps: Vec<String> = pkg
            .dependencies
            .iter()
            .filter(|d| d.kind == cargo_metadata::DependencyKind::Normal)
            .filter(|d| names.contains(&d.name.as_str()))
            .map(|d| d.name.clone())
            .collect();
        deps.sort();
        deps.dedup();
        deps
    };

    // ── L0: primitives ──────────────────────────────────────────────────
    assert!(
        ws_deps("codesight-core").is_empty(),
        "codesight-core must have no workspace deps"
    );
    assert_eq!(
        ws_deps("codesight-extractors"),
        vec!["codesight-core", "codesight-language-support"],
        "extractors may only depend on core + language-support (Phase C)"
    );
    assert_eq!(
        ws_deps("codesight-storage"),
        vec!["codesight-core"],
        "storage may only depend on core"
    );
    assert_eq!(
        ws_deps("codesight-analysis"),
        vec![
            "codesight-core",
            "codesight-language-support",
            "codesight-storage",
        ],
        "analysis depends on core + language-support + storage (Phase D)"
    );
    assert_eq!(
        ws_deps("codesight-imports"),
        vec![
            "codesight-core",
            "codesight-extractors",
            "codesight-language-support",
        ],
        "imports depends on core + extractors + language-support only"
    );
    assert_eq!(
        ws_deps("codesight-retrieval"),
        vec!["codesight-core", "codesight-storage"],
        "retrieval depends on core + storage only"
    );
    // ── L0.75: language support (LANGUAGES.md §2.3) ─────────────────────
    //
    // Owns every per-language behavior. May only depend on L0 primitives
    // (codesight-core). Must never reach upward into a concrete backend,
    // the engine, analysis, imports, extractors, framework, retrieval, or
    // storage — otherwise the "one crate owns language behavior" contract
    // collapses into the same scattered tables the refactor is removing.
    let lang_deps = ws_deps("codesight-language-support");
    for banned in [
        "codesight-analysis",
        "codesight-backend",
        "codesight-engine",
        "codesight-extractors",
        "codesight-imports",
        "codesight-retrieval",
        "codesight-storage",
        "mcp-rpc",
    ] {
        assert!(
            !lang_deps.contains(&banned.to_string()),
            "codesight-language-support must not depend on {banned}: {lang_deps:?}"
        );
    }

    // ── L0.5: backend trait + optional impls (Cargo features) ──────────
    let backend_deps = ws_deps("codesight-backend");
    assert!(
        backend_deps.contains(&"codesight-core".to_string()),
        "codesight-backend must depend on core: {backend_deps:?}"
    );
    assert!(
        backend_deps.contains(&"codesight-extractors".to_string()),
        "codesight-backend lists extractors for the tree-sitter feature: {backend_deps:?}"
    );
    for banned in ["codesight-engine", "codesight-storage"] {
        assert!(
            !backend_deps.contains(&banned.to_string()),
            "codesight-backend must not depend on {banned}: {backend_deps:?}"
        );
    }

    // ── L2: protocol ────────────────────────────────────────────────────
    let rpc_deps = ws_deps("mcp-rpc");
    for nope in ["codesight-core", "codesight-engine", "codesight-storage"] {
        assert!(
            !rpc_deps.contains(&nope.to_string()),
            "mcp-rpc must stay generic; found dep {nope}: {rpc_deps:?}"
        );
    }

    // ── L1: engine ──────────────────────────────────────────────────────
    let engine_deps = ws_deps("codesight-engine");
    for required in [
        "codesight-backend",
        "codesight-storage",
        "codesight-analysis",
        "codesight-retrieval",
        "codesight-language-support",
        "mcp-rpc",
    ] {
        assert!(
            engine_deps.contains(&required.to_string()),
            "engine missing required dep {required}: {engine_deps:?}"
        );
    }
    // Phase C gate (MEGAPLAN §7): engine must not reach for tree-sitter
    // directly. Routing happens through the backend trait.
    for banned in ["codesight-extractors"] {
        assert!(
            !engine_deps.contains(&banned.to_string()),
            "engine must not depend on extractors directly ({banned}): {engine_deps:?}"
        );
    }

    // No workspace crate may depend on the product binary crate.
    for pkg_name in &names {
        if *pkg_name == "codesight-mcp" {
            continue;
        }
        assert!(
            !ws_deps(pkg_name).contains(&"codesight-mcp".to_string()),
            "{pkg_name} must not depend on codesight-mcp"
        );
    }

    // Root binary depends on engine + backend (tree-sitter feature) + mcp-rpc.
    let root_deps = ws_deps("codesight-mcp");
    for required in [
        "codesight-engine",
        "codesight-backend",
        "mcp-rpc",
    ] {
        assert!(
            root_deps.contains(&required.to_string()),
            "root missing required dep {required}: {root_deps:?}"
        );
    }
}
