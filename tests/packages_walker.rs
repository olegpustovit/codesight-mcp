//! Integration tests for monorepo-aware manifest discovery.
//!
//! Moved out of `src/indexing/packages/walker.rs` as part of S1 test layering —
//! these tests only touch the public `discover_package_names` API and
//! `MAX_DEPTH` constant.

use std::fs;
use std::path::Path;

use codesight_mcp::indexing::packages::walker::{MAX_DEPTH, discover_package_names};
use tempfile::tempdir;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn cargo_toml(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
}

fn package_json(name: &str) -> String {
    format!(r#"{{"name": "{name}"}}"#)
}

// ── Monorepo layouts ──

#[test]
fn virtual_cargo_workspace_yields_member_crates() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    write(root, "crates/alpha/Cargo.toml", &cargo_toml("alpha"));
    write(root, "crates/beta/Cargo.toml", &cargo_toml("beta"));

    let names = discover_package_names(root);
    assert!(names.contains(&"alpha".to_string()), "got {names:?}");
    assert!(names.contains(&"beta".to_string()), "got {names:?}");
    assert!(!names.iter().any(String::is_empty));
}

#[test]
fn workspace_root_that_is_also_a_package_includes_both() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"root-crate\"\nversion = \"0.1.0\"\n",
    );
    write(root, "crates/child/Cargo.toml", &cargo_toml("child"));

    let names = discover_package_names(root);
    assert!(names.contains(&"root-crate".to_string()), "got {names:?}");
    assert!(names.contains(&"child".to_string()), "got {names:?}");
}

#[test]
fn npm_workspace_collects_root_and_members() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "package.json",
        r#"{"name": "root-pkg", "workspaces": ["packages/*"]}"#,
    );
    write(root, "packages/a/package.json", &package_json("@org/a"));
    write(root, "packages/b/package.json", &package_json("@org/b"));

    let names = discover_package_names(root);
    assert!(names.contains(&"root-pkg".to_string()), "got {names:?}");
    assert!(names.contains(&"@org/a".to_string()), "got {names:?}");
    assert!(names.contains(&"@org/b".to_string()), "got {names:?}");
}

#[test]
fn python_monorepo_with_multiple_pyproject_files() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "pyproject.toml", "[project]\nname = \"root-pkg\"\n");
    write(
        root,
        "libs/alpha/pyproject.toml",
        "[project]\nname = \"alpha\"\n",
    );
    write(
        root,
        "libs/beta/pyproject.toml",
        "[project]\nname = \"beta\"\n",
    );

    let names = discover_package_names(root);
    assert!(names.contains(&"root-pkg".to_string()), "got {names:?}");
    assert!(names.contains(&"alpha".to_string()), "got {names:?}");
    assert!(names.contains(&"beta".to_string()), "got {names:?}");
}

#[test]
fn go_multi_module_repo_collects_every_go_mod() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "go.mod", "module github.com/org/root\n");
    write(root, "services/api/go.mod", "module github.com/org/api\n");
    write(
        root,
        "services/worker/go.mod",
        "module github.com/org/worker\n",
    );

    let names = discover_package_names(root);
    assert!(names.contains(&"github.com/org/root".to_string()));
    assert!(names.contains(&"github.com/org/api".to_string()));
    assert!(names.contains(&"github.com/org/worker".to_string()));
}

#[test]
fn dotnet_solution_with_multiple_csproj_depths() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(
        root,
        "solution/App/App.csproj",
        "<Project><PropertyGroup><PackageName>MyApp</PackageName></PropertyGroup></Project>",
    );
    write(
        root,
        "solution/Lib/Lib.csproj",
        "<Project><PropertyGroup><AssemblyName>MyLib</AssemblyName></PropertyGroup></Project>",
    );

    let names = discover_package_names(root);
    assert!(names.contains(&"MyApp".to_string()), "got {names:?}");
    assert!(names.contains(&"MyLib".to_string()), "got {names:?}");
}

#[test]
fn mixed_ecosystems_in_one_monorepo() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    write(root, "crates/core/Cargo.toml", &cargo_toml("core"));
    write(root, "web/package.json", &package_json("@org/web"));
    write(
        root,
        "scripts/pyproject.toml",
        "[project]\nname = \"scripts\"\n",
    );
    write(root, "services/api/go.mod", "module github.com/org/api\n");
    write(
        root,
        "tools/Foo.csproj",
        "<Project><PropertyGroup><PackageName>Foo.Tools</PackageName></PropertyGroup></Project>",
    );

    let names = discover_package_names(root);
    assert!(names.contains(&"core".to_string()), "got {names:?}");
    assert!(names.contains(&"@org/web".to_string()), "got {names:?}");
    assert!(names.contains(&"scripts".to_string()), "got {names:?}");
    assert!(
        names.contains(&"github.com/org/api".to_string()),
        "got {names:?}"
    );
    assert!(names.contains(&"Foo.Tools".to_string()), "got {names:?}");
}

// ── Pruning ──

#[test]
fn node_modules_is_pruned() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "package.json", &package_json("host"));
    write(
        root,
        "node_modules/evil/package.json",
        &package_json("evil"),
    );
    write(
        root,
        "node_modules/deeper/inner/package.json",
        &package_json("nested-evil"),
    );

    let names = discover_package_names(root);
    assert!(names.contains(&"host".to_string()), "got {names:?}");
    assert!(!names.contains(&"evil".to_string()), "got {names:?}");
    assert!(!names.contains(&"nested-evil".to_string()), "got {names:?}");
}

#[test]
fn target_build_dist_out_vendor_bin_obj_are_pruned() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "Cargo.toml", &cargo_toml("real"));
    write(root, "target/tmp/Cargo.toml", &cargo_toml("stale-target"));
    write(root, "build/foo/Cargo.toml", &cargo_toml("stale-build"));
    write(root, "dist/pkg/package.json", &package_json("stale-dist"));
    write(root, "out/pkg/package.json", &package_json("stale-out"));
    write(root, "vendor/foo/go.mod", "module vendored.example/stale\n");
    write(
        root,
        "bin/Tool.csproj",
        "<Project><PropertyGroup><PackageName>StaleBin</PackageName></PropertyGroup></Project>",
    );
    write(
        root,
        "obj/Tool.csproj",
        "<Project><PropertyGroup><PackageName>StaleObj</PackageName></PropertyGroup></Project>",
    );

    let names = discover_package_names(root);
    assert_eq!(names, vec!["real".to_string()], "got {names:?}");
}

#[test]
fn hidden_and_vcs_dirs_are_pruned() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "Cargo.toml", &cargo_toml("visible"));
    write(root, ".git/tmp/Cargo.toml", &cargo_toml("git-stale"));
    write(root, ".cache/Cargo.toml", &cargo_toml("cache-stale"));
    write(
        root,
        ".venv/lib/pkg/pyproject.toml",
        "[project]\nname = \"venv-lib\"\n",
    );
    write(
        root,
        ".github/workflows/pkg/package.json",
        &package_json("github-stale"),
    );

    let names = discover_package_names(root);
    assert_eq!(names, vec!["visible".to_string()], "got {names:?}");
}

#[test]
fn respects_max_depth_cap() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();

    let mut too_deep = String::new();
    for _ in 0..(MAX_DEPTH + 2) {
        too_deep.push_str("d/");
    }
    too_deep.push_str("Cargo.toml");
    write(root, &too_deep, &cargo_toml("too-deep"));

    write(root, "shallow/Cargo.toml", &cargo_toml("shallow"));

    let names = discover_package_names(root);
    assert!(names.contains(&"shallow".to_string()), "got {names:?}");
    assert!(!names.contains(&"too-deep".to_string()), "got {names:?}");
}

#[cfg(unix)]
#[test]
fn does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let walk_root_tmp = tempdir().unwrap();
    let root = walk_root_tmp.path();
    write(root, "inside/Cargo.toml", &cargo_toml("inside"));

    let external_tmp = tempdir().unwrap();
    write(external_tmp.path(), "Cargo.toml", &cargo_toml("outside"));

    symlink(external_tmp.path(), root.join("linked")).unwrap();

    let names = discover_package_names(root);
    assert!(names.contains(&"inside".to_string()), "got {names:?}");
    assert!(
        !names.contains(&"outside".to_string()),
        "symlinks must not be followed, got {names:?}"
    );
}

// ── Dedup / ordering / robustness ──

#[test]
fn duplicate_names_are_collapsed() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "crates/a/Cargo.toml", &cargo_toml("shared"));
    write(root, "crates/b/Cargo.toml", &cargo_toml("shared"));

    let names = discover_package_names(root);
    assert_eq!(
        names.iter().filter(|n| *n == "shared").count(),
        1,
        "duplicates should collapse, got {names:?}"
    );
}

#[test]
fn discovery_order_is_stable_and_sorted_by_filename() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "crates/zeta/Cargo.toml", &cargo_toml("zeta"));
    write(root, "crates/alpha/Cargo.toml", &cargo_toml("alpha"));
    write(root, "crates/mu/Cargo.toml", &cargo_toml("mu"));

    let first = discover_package_names(root);
    let second = discover_package_names(root);
    assert_eq!(first, second, "discovery must be deterministic");

    let idx_alpha = first.iter().position(|n| n == "alpha").unwrap();
    let idx_mu = first.iter().position(|n| n == "mu").unwrap();
    let idx_zeta = first.iter().position(|n| n == "zeta").unwrap();
    assert!(idx_alpha < idx_mu && idx_mu < idx_zeta, "got {first:?}");
}

#[test]
fn malformed_manifests_do_not_panic() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();
    write(root, "pyproject.toml", "][[[ not toml");
    write(root, "package.json", "{broken");
    write(root, "go.mod", "not a go mod file\n");
    write(root, "Cargo.toml", "[[[ not toml");
    write(root, "a.csproj", "<Project><unterminated");

    let names = discover_package_names(root);
    assert!(
        names.is_empty(),
        "malformed manifests should yield nothing, got {names:?}"
    );
}

// ── Degenerate inputs ──

#[test]
fn nonexistent_root_returns_empty() {
    let names = discover_package_names(Path::new("/nonexistent/a/b/c"));
    assert!(names.is_empty());
}

#[test]
fn file_instead_of_dir_returns_empty() {
    let tmp = tempdir().unwrap();
    let file_path = tmp.path().join("not-a-dir.txt");
    fs::write(&file_path, "hi").unwrap();
    let names = discover_package_names(&file_path);
    assert!(names.is_empty());
}

#[test]
fn empty_dir_returns_empty() {
    let tmp = tempdir().unwrap();
    let names = discover_package_names(tmp.path());
    assert!(names.is_empty());
}
