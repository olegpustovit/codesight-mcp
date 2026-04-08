use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use codesight_mcp::extractors::ExtractorRegistry;
use codesight_mcp::extractors::engine::catalogs::QueryPackCatalog;
use codesight_mcp::extractors::engine::scm::ScmExtractor;

fn query_dirs_on_disk() -> BTreeSet<String> {
    let queries_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/queries");
    fs::read_dir(queries_root)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let dir = entry.path();
            if dir.join("tags.scm").is_file() {
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
fn builtin_query_packs_use_canonical_tags_layout() {
    let queries_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/queries");
    let legacy_paths: Vec<String> = fs::read_dir(queries_root)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path().join("symbols.scm");
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
fn workspace_query_pack_root_supports_pack_toml_metadata() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".codesight.toml"),
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

    let registry = ExtractorRegistry::for_workspace_root(workspace.path());
    let extractor = registry
        .extractor_for_path(Path::new("example.toyjson"))
        .expect("workspace extractor");
    assert_eq!(extractor.language_name(), "toyjson");

    let symbols = extractor
        .extract("example.toyjson", r#"{"title":"hello"}"#)
        .unwrap();
    assert!(symbols.iter().any(|symbol| symbol.name == "title"));
}

#[test]
fn workspace_query_pack_missing_required_metadata_is_rejected() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(
        workspace.path().join(".codesight.toml"),
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
