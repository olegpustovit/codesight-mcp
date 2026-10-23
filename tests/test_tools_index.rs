//! Integration tests: test tools index.

use dowser_engine::github_index::{
    DiscoverOpts, discover_github_tree_source_files, parse_github_url, should_skip_file_path,
};

#[test]
fn tools_github_tree_suite() {
    case_parse_github_url_full();
    case_parse_github_url_with_git();
    case_parse_github_url_short();
    case_should_skip_file();
    case_discover_source_files();
    case_discover_source_files_respects_max();
    case_discover_source_files_prioritizes_src();
    case_discover_source_files_uses_config_override();
    case_discover_source_files_explicit_max_overrides_config();
    case_discover_source_files_exact_limit_is_not_truncated();
}

fn case_parse_github_url_full() {
    assert_eq!(
        parse_github_url("https://github.com/owner/repo"),
        Some(("owner".into(), "repo".into()))
    );
}

fn case_parse_github_url_with_git() {
    assert_eq!(
        parse_github_url("https://github.com/owner/repo.git"),
        Some(("owner".into(), "repo".into()))
    );
}

fn case_parse_github_url_short() {
    assert_eq!(
        parse_github_url("owner/repo"),
        Some(("owner".into(), "repo".into()))
    );
}

fn case_should_skip_file() {
    assert!(should_skip_file_path("node_modules/foo.js"));
    assert!(should_skip_file_path("vendor/github.com/foo.go"));
    assert!(!should_skip_file_path("src/main.py"));
}

fn case_discover_source_files() {
    let paths = [
        "src/main.py",
        "node_modules/foo.js",
        "README.md",
        "src/utils.py",
        "src/engine.cpp",
        "include/engine.hpp",
    ];
    let (files, _total, truncated) =
        discover_github_tree_source_files(&paths, &DiscoverOpts::default());
    assert!(files.contains(&"src/main.py".to_string()));
    assert!(files.contains(&"src/utils.py".to_string()));
    assert!(files.contains(&"src/engine.cpp".to_string()));
    assert!(files.contains(&"include/engine.hpp".to_string()));
    assert!(!files.iter().any(|p| p.contains("node_modules")));
    assert!(!files.iter().any(|p| p.ends_with("README.md")));
    assert!(!truncated);
}

fn case_discover_source_files_respects_max() {
    let paths: Vec<String> = (0..1000).map(|i| format!("file{i}.py")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (files, _, truncated) = discover_github_tree_source_files(
        &refs,
        &DiscoverOpts {
            explicit_max_files: Some(100),
            ..Default::default()
        },
    );
    assert_eq!(files.len(), 100);
    assert!(truncated);
}

fn case_discover_source_files_prioritizes_src() {
    let mut owned: Vec<String> = (0..300).map(|i| format!("other/file{i}.py")).collect();
    owned.extend((0..300).map(|i| format!("src/file{i}.py")));
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    let (files, _, _) = discover_github_tree_source_files(
        &refs,
        &DiscoverOpts {
            explicit_max_files: Some(100),
            ..Default::default()
        },
    );
    let src_count = files.iter().filter(|p| p.starts_with("src/")).count();
    assert!(src_count > 50);
}

fn case_discover_source_files_uses_config_override() {
    let paths: Vec<String> = (0..20).map(|i| format!("file{i}.py")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (files, _, truncated) = discover_github_tree_source_files(
        &refs,
        &DiscoverOpts {
            config_max_files: Some(7),
            ..Default::default()
        },
    );
    assert_eq!(files.len(), 7);
    assert!(truncated);
}

fn case_discover_source_files_explicit_max_overrides_config() {
    let paths: Vec<String> = (0..20).map(|i| format!("file{i}.py")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (files, _, truncated) = discover_github_tree_source_files(
        &refs,
        &DiscoverOpts {
            config_max_files: Some(7),
            explicit_max_files: Some(5),
        },
    );
    assert_eq!(files.len(), 5);
    assert!(truncated);
}

fn case_discover_source_files_exact_limit_is_not_truncated() {
    let paths: Vec<String> = (0..5).map(|i| format!("file{i}.py")).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (files, _, truncated) = discover_github_tree_source_files(
        &refs,
        &DiscoverOpts {
            explicit_max_files: Some(5),
            ..Default::default()
        },
    );
    assert_eq!(files.len(), 5);
    assert!(!truncated);
}
