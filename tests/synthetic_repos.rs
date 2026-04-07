//! Synthetic multi-file Python repo layouts for filesystem-only checks.

use codesight_mcp::test_support::{write_hierarchy_repo, write_medium_repo, write_small_repo};
use tempfile::tempdir;

#[test]
fn small_repo_files_on_disk() {
    let d = tempdir().unwrap();
    write_small_repo(d.path()).unwrap();
    let py = std::fs::read_to_string(d.path().join("utils.py")).unwrap();
    assert!(py.contains("MAX_RETRIES"));
    assert!(py.contains("def add"));
    assert!(py.contains("def subtract"));
}

#[test]
fn medium_repo_three_modules() {
    let d = tempdir().unwrap();
    write_medium_repo(d.path()).unwrap();
    for f in ["models.py", "service.py", "api.py"] {
        assert!(d.path().join(f).is_file(), "missing {f}");
    }
}

#[test]
fn hierarchy_four_classes() {
    let d = tempdir().unwrap();
    write_hierarchy_repo(d.path()).unwrap();
    let src = std::fs::read_to_string(d.path().join("animals.py")).unwrap();
    for cls in ["Animal", "Mammal", "Dog", "Cat"] {
        assert!(src.contains(&format!("class {cls}")), "missing class {cls}");
    }
}
