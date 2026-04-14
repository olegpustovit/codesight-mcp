//! CI gate: assert the workspace crate dependency DAG is frozen.

#[test]
fn crate_dependency_graph_is_frozen() {
    let metadata = cargo_metadata::MetadataCommand::new().exec().unwrap();
    let workspace_members: Vec<&cargo_metadata::Package> = metadata
        .packages
        .iter()
        .filter(|p| metadata.workspace_members.contains(&p.id))
        .collect();

    // Exactly 4 workspace members
    let names: Vec<&str> = workspace_members.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names.len(),
        4,
        "expected 4 workspace crates, got: {names:?}"
    );
    for expected in [
        "codesight-mcp",
        "codesight-core",
        "codesight-extractors",
        "codesight-storage",
    ] {
        assert!(
            names.contains(&expected),
            "missing workspace crate: {expected}"
        );
    }

    // Helper: get workspace deps of a package
    let ws_deps = |pkg_name: &str| -> Vec<String> {
        let pkg = workspace_members
            .iter()
            .find(|p| p.name == pkg_name)
            .unwrap();
        pkg.dependencies
            .iter()
            .filter(|d| names.contains(&d.name.as_str()))
            .map(|d| d.name.clone())
            .collect::<Vec<_>>()
    };

    // codesight-core has no workspace deps
    assert!(
        ws_deps("codesight-core").is_empty(),
        "codesight-core must have no workspace deps"
    );

    // codesight-extractors depends only on codesight-core
    let ext_deps = ws_deps("codesight-extractors");
    assert_eq!(
        ext_deps,
        vec!["codesight-core"],
        "extractors deps: {ext_deps:?}"
    );

    // codesight-storage depends only on codesight-core
    let stor_deps = ws_deps("codesight-storage");
    assert_eq!(
        stor_deps,
        vec!["codesight-core"],
        "storage deps: {stor_deps:?}"
    );

    // codesight-extractors does NOT depend on codesight-storage
    assert!(!ws_deps("codesight-extractors").contains(&"codesight-storage".to_string()));
    // codesight-storage does NOT depend on codesight-extractors
    assert!(!ws_deps("codesight-storage").contains(&"codesight-extractors".to_string()));

    // Root crate depends on all three
    let root_deps = ws_deps("codesight-mcp");
    for dep in [
        "codesight-core",
        "codesight-extractors",
        "codesight-storage",
    ] {
        assert!(
            root_deps.contains(&dep.to_string()),
            "root missing dep: {dep}"
        );
    }
}
