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
    let required = "codesight-engine";
    assert!(
        root_deps.contains(&required.to_string()),
        "root missing required dep {required}: {root_deps:?}"
    );
}

/// L0.75 content lint: per-language behavior lives in
/// `codesight-language-support`. The analysis and engine crates must not
/// branch on language-name string literals or on raw source-file extensions
/// — both belong behind a `Language` trait method.
#[test]
fn analysis_and_engine_have_no_per_language_branches() {
    use std::fs;
    use std::path::Path;

    // Match arms keyed on language identifiers / source-file extensions.
    const BANNED: &[&str] = &[
        "\"python\" =>",
        "\"rust\" =>",
        "\"javascript\" =>",
        "\"typescript\" =>",
        "\"ruby\" =>",
        "\"go\" =>",
        "\"php\" =>",
        "\"java\" =>",
        "\"kotlin\" =>",
        "\"swift\" =>",
        "\"elixir\" =>",
        "\".py\" =>",
        "\".rs\" =>",
        "\".go\" =>",
        "\".ts\" =>",
        "\".js\" =>",
        "\".rb\" =>",
        ".ends_with(\".py\")",
        ".ends_with(\".rs\")",
        ".ends_with(\".go\")",
        ".ends_with(\".ts\")",
        ".ends_with(\".js\")",
        ".ends_with(\".rb\")",
    ];

    fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    let mut offenders: Vec<String> = Vec::new();
    for root in [
        "crates/codesight-analysis/src",
        "crates/codesight-engine/src",
    ] {
        let mut files = Vec::new();
        walk(Path::new(root), &mut files);
        for file in files {
            let Ok(content) = fs::read_to_string(&file) else {
                continue;
            };
            for pat in BANNED {
                if let Some(pos) = content.find(pat) {
                    // Skip comment lines to reduce false positives on docs.
                    let line_start = content[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
                    let line = &content[line_start
                        ..content[line_start..]
                            .find('\n')
                            .map(|e| line_start + e)
                            .unwrap_or(content.len())];
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    offenders.push(format!("{}: {}", file.display(), pat));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "per-language branches found in analysis/engine — push to Language trait:\n  {}",
        offenders.join("\n  ")
    );
}
