//! CI gate: assert the workspace crate dependency invariants still hold.
//!
//! Encodes layering: L0 primitives → L0.75 language-support (owns every
//! per-language heuristic) → L1 engine → L2 protocol. Each layer may only
//! depend on layers below it.

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
        "codesight-core",
        "codesight-engine",
        "codesight-language-support",
        "codesight-retrieval",
        "codesight-storage",
    ] {
        assert!(
            names.contains(&expected),
            "missing workspace crate: {expected}"
        );
    }
    for gone in [
        "codesight-backend",
        "codesight-extractors",
        "codesight-imports",
        "mcp-rpc",
    ] {
        assert!(
            !names.contains(&gone),
            "{gone} has been deleted; tree-sitter extraction flows through \
             codesight-language-support directly"
        );
    }

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
        "analysis depends on core + language-support + storage"
    );
    assert_eq!(
        ws_deps("codesight-retrieval"),
        vec!["codesight-core", "codesight-storage"],
        "retrieval depends on core + storage only"
    );

    // ── L0.75: language support — owns every per-language behavior ──────
    //
    // May depend only on L0 primitives (codesight-core). Must never reach
    // upward into engine, analysis, retrieval, or storage — otherwise the
    // "one crate owns language behavior" contract collapses.
    let lang_deps = ws_deps("codesight-language-support");
    for banned in [
        "codesight-analysis",
        "codesight-engine",
        "codesight-retrieval",
        "codesight-storage",
    ] {
        assert!(
            !lang_deps.contains(&banned.to_string()),
            "codesight-language-support must not depend on {banned}: {lang_deps:?}"
        );
    }

    // ── L1: engine ──────────────────────────────────────────────────────
    let engine_deps = ws_deps("codesight-engine");
    for required in [
        "codesight-storage",
        "codesight-analysis",
        "codesight-retrieval",
        "codesight-language-support",
    ] {
        assert!(
            engine_deps.contains(&required.to_string()),
            "engine missing required dep {required}: {engine_deps:?}"
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

    // Root binary depends on engine. The MCP protocol layer is now `rmcp`
    // (external crate), so there's no internal workspace member for it.
    let root_deps = ws_deps("codesight-mcp");
    for required in ["codesight-engine"] {
        assert!(
            root_deps.contains(&required.to_string()),
            "root missing required dep {required}: {root_deps:?}"
        );
    }
}
