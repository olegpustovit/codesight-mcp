//! Integration tests: path map.

use codesight_engine::path_map::{parse_path_map, remap};

// ===========================================================================
// parse_path_map tests (9 tests)
// ===========================================================================

#[test]
fn parse_path_map_unset() {
    // Env var not set -> empty list (we test the parse function directly with "")
    assert_eq!(parse_path_map(""), Vec::<(String, String)>::new());
}

#[test]
fn parse_path_map_whitespace_only() {
    assert_eq!(parse_path_map("   "), Vec::<(String, String)>::new());
}

#[test]
fn parse_path_map_single_pair() {
    assert_eq!(
        parse_path_map("/home/user=/mnt/user"),
        vec![("/home/user".to_owned(), "/mnt/user".to_owned())]
    );
}

#[test]
fn parse_path_map_multiple_pairs() {
    assert_eq!(
        parse_path_map("/a=/b,/c=/d"),
        vec![
            ("/a".to_owned(), "/b".to_owned()),
            ("/c".to_owned(), "/d".to_owned()),
        ]
    );
}

#[test]
fn parse_path_map_equals_in_path() {
    // Last = is the separator; earlier = chars belong to the original path.
    assert_eq!(
        parse_path_map("/home/user/a=b=/new/path"),
        vec![("/home/user/a=b".to_owned(), "/new/path".to_owned())]
    );
}

#[test]
fn parse_path_map_malformed_no_equals_skipped() {
    // "noequalssign" is skipped; valid pairs are kept.
    let result = parse_path_map("/valid=/ok,noequalssign,/also=/fine");
    assert_eq!(
        result,
        vec![
            ("/valid".to_owned(), "/ok".to_owned()),
            ("/also".to_owned(), "/fine".to_owned()),
        ]
    );
}

#[test]
fn parse_path_map_empty_orig_skipped() {
    // "=/new/path" -> empty original -> skipped
    let result = parse_path_map("=/new/path");
    assert!(result.is_empty());
}

#[test]
fn parse_path_map_empty_new_skipped() {
    // "/old/path=" -> empty replacement -> skipped
    let result = parse_path_map("/old/path=");
    assert!(result.is_empty());
}

#[test]
fn parse_path_map_whitespace_stripped() {
    // Leading/trailing whitespace in tokens is stripped.
    assert_eq!(
        parse_path_map(" /home/user = /mnt/user "),
        vec![("/home/user", "/mnt/user")]
            .into_iter()
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
// remap tests (8 tests)
// ===========================================================================

#[test]
fn remap_empty_pairs_returns_unchanged() {
    let path = "/home/user/project/file.py";
    assert_eq!(remap(path, &[], false), path);
}

#[test]
fn remap_forward_replaces_prefix() {
    let pairs = vec![("/home/user".to_owned(), "C:\\Users\\user".to_owned())];
    let result = remap("/home/user/project/file.py", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "C:/Users/user/project/file.py");
}

#[test]
fn remap_reverse_replaces_prefix() {
    let pairs = vec![("/home/user".to_owned(), "C:\\Users\\user".to_owned())];
    let result = remap("C:\\Users\\user\\project\\file.py", &pairs, true);
    assert_eq!(result.replace('\\', "/"), "/home/user/project/file.py");
}

#[test]
fn remap_no_match_returns_normalised() {
    let pairs = vec![("/home/other".to_owned(), "/mnt/other".to_owned())];
    let result = remap("/home/user/project", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "/home/user/project");
}

#[test]
fn remap_first_pair_wins() {
    let pairs = vec![
        ("/home/user".to_owned(), "/first".to_owned()),
        ("/home/user".to_owned(), "/second".to_owned()),
    ];
    let result = remap("/home/user/file.py", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "/first/file.py");
}

#[test]
fn remap_mixed_separators_in_input_match() {
    // D:/Users/user (forward slashes) matches stored prefix D:\\Users\\user.
    let pairs = vec![("/home/user".to_owned(), "D:\\Users\\user".to_owned())];
    let result = remap("D:/Users/user/project", &pairs, true);
    assert_eq!(result.replace('\\', "/"), "/home/user/project");
}

#[test]
fn remap_multiple_pairs_correct_one_matches() {
    let pairs = vec![
        ("/home/alice".to_owned(), "/mnt/alice".to_owned()),
        ("/home/bob".to_owned(), "/mnt/bob".to_owned()),
    ];
    let result = remap("/home/bob/work/file.py", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "/mnt/bob/work/file.py");
}

#[test]
fn remap_does_not_match_across_directory_boundary() {
    let pairs = vec![("/home/user".to_owned(), "/mnt/user".to_owned())];
    let result = remap("/home/username/file.py", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "/home/username/file.py");
}

// ===========================================================================
// Integration tests: list_repos, index_folder, watcher remap (5 tests)
// ===========================================================================

#[test]
fn list_repos_source_root_remapped() {
    use codesight_engine::config::AppConfig;
    
    use serde_json::json;

    let workspace = tempfile::tempdir().expect("tempdir");
    // Canonicalize to get the real path (macOS: /var -> /private/var)
    let workspace_canon = workspace
        .path()
        .canonicalize()
        .expect("canonicalize workspace");
    let repo_root = workspace_canon.join("myproject");
    std::fs::create_dir_all(&repo_root).unwrap();
    std::fs::write(repo_root.join("hello.py"), "def hello(): pass\n").unwrap();

    let index_root = workspace_canon.join(".index");
    let mut config = AppConfig::new(index_root.clone());
    // No path_map during indexing.
    let service = codesight_mcp::test_support::make_service_from_config(config.clone()).expect("service");

    let indexed = service
        .dispatch(
            "ingest_sources",
            json!({
                "path": repo_root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .expect("indexing");
    assert!(
        indexed["ok"].as_bool().unwrap_or(false),
        "index failed: {indexed}"
    );

    // Now set path_map: real prefix -> /remapped/prefix
    // Use the canonicalized workspace path as the original prefix.
    let orig_prefix = workspace_canon.to_string_lossy().to_string();
    config.path_map = vec![(orig_prefix.clone(), "/remapped/prefix".to_owned())];
    let service2 = codesight_mcp::test_support::make_service_from_config(config).expect("service2");

    let catalog = service2
        .dispatch("catalog_sources", json!({}))
        .expect("catalog");
    assert!(catalog["ok"].as_bool().unwrap_or(false));
    let sources = catalog["data"]["sources"]
        .as_array()
        .expect("sources array");
    assert!(!sources.is_empty(), "expected at least one source");
    let root_hint = sources[0]["root_hint"].as_str().unwrap_or("");
    assert!(
        root_hint.replace('\\', "/").starts_with("/remapped/prefix"),
        "Expected remapped prefix in root_hint, got: {root_hint} (orig_prefix={orig_prefix})"
    );
}

#[test]
fn repo_entry_from_data_source_root_remapped() {
    // _repo_entry_from_data is a Python-internal concept. In Rust we test that
    // catalog_sources applies forward remap to the root_hint (same as above
    // but with a specific stored root). This test verifies the remap function
    // directly on a stored path.
    let pairs = vec![("/old/root".to_owned(), "/new/root".to_owned())];
    let result = remap("/old/root/myproject", &pairs, false);
    assert_eq!(result.replace('\\', "/"), "/new/root/myproject");
}

#[test]
fn index_folder_reverse_remap_finds_existing_index() {
    // Verify that reverse remap produces the same repo_id hash as the original
    // path, so an existing index can be found when the user's path has been
    // remapped.
    use codesight_engine::repo_id::normalize_repo_id;
    use std::path::Path;

    let original_path = "/real/projects/myapp";
    let original_id = normalize_repo_id(Path::new(original_path));

    let pairs = vec![("/real/projects".to_owned(), "/docker/projects".to_owned())];
    let remapped_input = "/docker/projects/myapp";
    let reverse_remapped = remap(remapped_input, &pairs, true);
    let lookup_id = normalize_repo_id(Path::new(&reverse_remapped));

    assert_eq!(
        lookup_id, original_id,
        "Reverse-remapped path must produce same repo ID as stored path. \
         reverse_remapped={reverse_remapped:?}, expected path={original_path:?}"
    );
}

#[test]
fn watcher_reverse_remap_produces_stored_hash() {
    // Applying reverse remap before normalize_repo_id yields the same hash
    // as the stored path.
    use codesight_engine::repo_id::normalize_repo_id;
    use std::path::Path;

    let pairs = vec![("/real/project".to_owned(), "/remapped/project".to_owned())];

    let stored_id = normalize_repo_id(Path::new("/real/project"));
    let reverse = remap("/remapped/project", &pairs, true);
    let lookup_id = normalize_repo_id(Path::new(&reverse));

    assert_eq!(
        lookup_id, stored_id,
        "Reverse-remapped path must produce same hash as stored path. \
         Expected {stored_id:?}, got {lookup_id:?}"
    );
}

#[test]
fn stop_watching_remap_produces_stored_hash() {
    // The reverse remap produces the same hash as the original indexed path.
    use codesight_engine::repo_id::normalize_repo_id;
    use std::path::Path;

    let pairs = vec![("/stored/root".to_owned(), "/current/root".to_owned())];

    let original_id = normalize_repo_id(Path::new("/stored/root/myproject"));
    let reverse = remap("/current/root/myproject", &pairs, true);
    let cleanup_id = normalize_repo_id(Path::new(&reverse));

    assert_eq!(
        cleanup_id, original_id,
        "_stop_watching reverse remap must match stored hash. \
         Expected {original_id:?}, got {cleanup_id:?}"
    );
}
