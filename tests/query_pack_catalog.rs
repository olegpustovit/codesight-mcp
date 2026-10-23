use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use dowser_language_support::LanguageRegistry;
use dowser_language_support::query_pack_catalog::QueryPackCatalog;
use dowser_language_support::scm_engine::ScmExtractor;

fn query_dirs_on_disk() -> BTreeSet<String> {
    let languages_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/dowser-language-support/src/languages");
    fs::read_dir(languages_root)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let dir = entry.path();
            if dir.join("queries/tags.scm").is_file() {
                Some(entry.file_name().to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn builtin_catalog_matches_query_directories_on_disk() {
    let expected = query_dirs_on_disk();
    let actual: BTreeSet<String> = QueryPackCatalog::builtin()
        .iter()
        .map(|pack| pack.pack_id.clone())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn builtin_query_packs_compile_from_catalog() {
    let failures: Vec<String> = QueryPackCatalog::builtin()
        .iter()
        .filter(|pack| {
            pack.metadata.override_extractor.is_none() && !pack.metadata.grammar.is_empty()
        })
        .filter_map(|pack| {
            ScmExtractor::from_pack(pack)
                .err()
                .map(|error| format!("{}: {error:#}", pack.pack_id))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "failed to compile built-in query packs:\n{}",
        failures.join("\n")
    );
}

#[test]
fn override_backed_query_packs_still_compile_for_wrapper_delegation() {
    let failures: Vec<String> = QueryPackCatalog::builtin()
        .iter()
        .filter(|pack| {
            pack.metadata.override_extractor.is_some() && !pack.metadata.grammar.is_empty()
        })
        .filter_map(|pack| {
            ScmExtractor::from_pack(pack)
                .err()
                .map(|error| format!("{}: {error:#}", pack.pack_id))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "failed to compile override-backed query packs:\n{}",
        failures.join("\n")
    );
}

#[test]
fn builtin_query_packs_use_canonical_tags_layout() {
    let languages_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/dowser-language-support/src/languages");
    let legacy_paths: Vec<String> = fs::read_dir(languages_root)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path().join("queries/symbols.scm");
            path.is_file().then(|| path.display().to_string())
        })
        .collect();
    assert!(
        legacy_paths.is_empty(),
        "legacy symbols.scm packs remain:\n{}",
        legacy_paths.join("\n")
    );
}

#[test]
fn dowser_queries_convention_loads_packs_without_toml_config() {
    let workspace = tempfile::tempdir().unwrap();
    let pack_dir = workspace.path().join(".dowser/queries/toyjson");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(
        pack_dir.join("tags.scm"),
        "(document (object (pair key: (string (string_content) @name)) @definition.constant))\n",
    )
    .unwrap();
    fs::write(
        pack_dir.join("pack.toml"),
        "language = \"toyjson\"\nextensions = [\"toyjson\"]\ngrammar = \"json\"\n",
    )
    .unwrap();

    let catalog = QueryPackCatalog::for_workspace_root(workspace.path()).unwrap();
    let pack = catalog.find_by_pack_id("toyjson").expect("toyjson pack");
    assert_eq!(pack.metadata.name, "toyjson");

    let registry = LanguageRegistry::for_workspace_root(workspace.path()).unwrap();
    let extractor = registry
        .extractor_for_path(Path::new("example.toyjson"))
        .expect("workspace extractor");
    assert_eq!(extractor.id().as_str(), "toyjson");
}

#[test]
fn workspace_query_pack_root_supports_pack_toml_metadata() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".dowser.toml"),
        "query_pack_roots = [\"queries\"]\n",
    )
    .unwrap();

    let pack_dir = workspace.path().join("queries/toyjson");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(
        pack_dir.join("tags.scm"),
        "(document (object (pair key: (string (string_content) @name)) @definition.constant))\n",
    )
    .unwrap();
    fs::write(
        pack_dir.join("pack.toml"),
        "language = \"toyjson\"\nextensions = [\"toyjson\"]\ngrammar = \"json\"\n",
    )
    .unwrap();

    let catalog = QueryPackCatalog::for_workspace_root(workspace.path()).unwrap();
    let pack = catalog.find_by_pack_id("toyjson").expect("toyjson pack");
    assert_eq!(pack.metadata.name, "toyjson");
    assert_eq!(pack.metadata.extensions, vec!["toyjson"]);
    assert_eq!(pack.metadata.grammar, "json");

    let registry = LanguageRegistry::for_workspace_root(workspace.path()).unwrap();
    let extractor = registry
        .extractor_for_path(Path::new("example.toyjson"))
        .expect("workspace extractor");
    assert_eq!(extractor.id().as_str(), "toyjson");

    let symbols = extractor
        .extract_symbols("example.toyjson", r#"{"title":"hello"}"#)
        .unwrap();
    assert!(symbols.iter().any(|symbol| symbol.name == "title"));
}

#[test]
fn workspace_query_pack_missing_required_metadata_is_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".dowser.toml"),
        "query_pack_roots = [\"queries\"]\n",
    )
    .unwrap();

    let pack_dir = workspace.path().join("queries/broken");
    fs::create_dir_all(&pack_dir).unwrap();
    fs::write(
        pack_dir.join("tags.scm"),
        "(document (object (pair key: (string (string_content) @name)) @definition.constant))\n",
    )
    .unwrap();

    let error = QueryPackCatalog::for_workspace_root(workspace.path())
        .err()
        .expect("workspace pack should fail validation");
    let message = format!("{error:#}");
    assert!(
        message.contains("missing extensions metadata")
            || message.contains("must declare either grammar or override_extractor"),
        "unexpected error: {message}"
    );
}
