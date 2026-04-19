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
        "codesight-backend-lsp",
        "codesight-backend-treesitter",
        "codesight-core",
        "codesight-engine",
        "codesight-extractors",
        "codesight-framework",
        "codesight-imports",
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
        vec!["codesight-core"],
        "extractors may only depend on core"
    );
    assert_eq!(
        ws_deps("codesight-storage"),
        vec!["codesight-core"],
        "storage may only depend on core"
    );
    assert_eq!(
        ws_deps("codesight-analysis"),
        vec!["codesight-core", "codesight-storage"],
        "analysis depends on core + storage only"
    );
    assert_eq!(
        ws_deps("codesight-imports"),
        vec!["codesight-core", "codesight-extractors"],
        "imports depends on core + extractors only"
    );
    assert_eq!(
        ws_deps("codesight-retrieval"),
        vec!["codesight-core", "codesight-storage"],
        "retrieval depends on core + storage only"
    );
    assert!(
        ws_deps("codesight-framework").contains(&"codesight-extractors".to_string()),
        "framework needs extractors for parse_tree"
    );
    assert!(
        !ws_deps("codesight-framework").contains(&"codesight-storage".to_string()),
        "framework must not depend on storage"
    );

    // ── L0.5: backend trait + tree-sitter impl ──────────────────────────
    assert_eq!(
        ws_deps("codesight-backend"),
        vec!["codesight-core"],
        "backend trait depends only on core"
    );
    let ts_deps = ws_deps("codesight-backend-treesitter");
    assert!(
        ts_deps.contains(&"codesight-backend".to_string()),
        "backend-treesitter must depend on the trait: {ts_deps:?}"
    );
    assert!(
        ts_deps.contains(&"codesight-extractors".to_string()),
        "backend-treesitter wraps extractors: {ts_deps:?}"
    );
    assert!(
        !ts_deps.contains(&"codesight-engine".to_string()),
        "backend impls must not depend on the engine: {ts_deps:?}"
    );
    assert!(
        !ts_deps.contains(&"codesight-storage".to_string()),
        "backend-treesitter must not reach into storage directly: {ts_deps:?}"
    );

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
        "codesight-framework",
        "mcp-rpc",
    ] {
        assert!(
            engine_deps.contains(&required.to_string()),
            "engine missing required dep {required}: {engine_deps:?}"
        );
    }
    // Phase C gate (MEGAPLAN §7): engine must not reach for tree-sitter
    // directly. Routing happens through the backend trait.
    for banned in ["codesight-backend-treesitter", "codesight-backend-lsp", "codesight-extractors"] {
        assert!(
            !engine_deps.contains(&banned.to_string()),
            "engine must not depend on a concrete backend impl ({banned}): {engine_deps:?}"
        );
    }

    // ── L0.5: LSP backend ───────────────────────────────────────────────
    let lsp_deps = ws_deps("codesight-backend-lsp");
    assert_eq!(
        lsp_deps,
        vec!["codesight-backend"],
        "LSP backend depends only on the trait crate"
    );

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

    // Root binary depends on engine + backend-treesitter + mcp-rpc.
    let root_deps = ws_deps("codesight-mcp");
    for required in [
        "codesight-engine",
        "codesight-backend-treesitter",
        "mcp-rpc",
    ] {
        assert!(
            root_deps.contains(&required.to_string()),
            "root missing required dep {required}: {root_deps:?}"
        );
    }
}
