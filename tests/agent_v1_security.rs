//! Integration tests: agent v1 security.

use std::fs;
use std::os::unix::fs as unix_fs;

use tempfile::tempdir;

use codesight_mcp::security::*;

// ── Path Traversal Prevention (S-01) ────────────────────────────────────────

#[test]
fn path_traversal_valid_path_within_root() {
    let dir = tempdir().unwrap();
    let child = dir.path().join("src").join("main.py");
    fs::create_dir_all(child.parent().unwrap()).unwrap();
    fs::write(&child, "").unwrap();

    assert!(validate_path(dir.path(), &child));
}

#[test]
fn path_traversal_dot_dot_blocked() {
    let dir = tempdir().unwrap();
    let evil = dir.path().join("..").join("..").join("etc").join("passwd");

    assert!(!validate_path(dir.path(), &evil));
}

#[test]
fn path_traversal_root_itself_is_valid() {
    let dir = tempdir().unwrap();

    assert!(validate_path(dir.path(), dir.path()));
}

#[test]
fn path_traversal_deeply_nested_valid() {
    let dir = tempdir().unwrap();
    let deep = dir
        .path()
        .join("a")
        .join("b")
        .join("c")
        .join("d")
        .join("file.py");
    fs::create_dir_all(deep.parent().unwrap()).unwrap();
    fs::write(&deep, "").unwrap();

    assert!(validate_path(dir.path(), &deep));
}

#[test]
fn path_traversal_sibling_directory_blocked() {
    let dir = tempdir().unwrap();
    let sibling = dir
        .path()
        .parent()
        .unwrap()
        .join("other_project")
        .join("secret.py");

    assert!(!validate_path(dir.path(), &sibling));
}

// ── Symlink Escape Protection (S-02) ────────────────────────────────────────

#[test]
fn symlink_inside_root_ok() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("real.py");
    fs::write(&target, "").unwrap();
    let link = dir.path().join("link.py");
    unix_fs::symlink(&target, &link).unwrap();

    assert!(!is_symlink_escape(dir.path(), &link));
}

#[test]
fn symlink_outside_root_blocked() {
    let dir = tempdir().unwrap();
    let outside = dir.path().parent().unwrap().join("outside.py");
    fs::write(&outside, "outside").unwrap();
    let link = dir.path().join("escape.py");
    unix_fs::symlink(&outside, &link).unwrap();

    assert!(is_symlink_escape(dir.path(), &link));
}

#[test]
fn non_symlink_not_escape() {
    let dir = tempdir().unwrap();
    let regular = dir.path().join("regular.py");
    fs::write(&regular, "").unwrap();

    assert!(!is_symlink_escape(dir.path(), &regular));
}

// ── Secret Detection (S-04) ─────────────────────────────────────────────────

#[test]
fn secret_files_detected() {
    let secret_names = [
        ".env",
        "config.env",
        ".env.local",
        ".env.production",
        "server.pem",
        "private.key",
        "cert.p12",
        "id_rsa",
        "id_rsa.pub",
        "id_ed25519",
        "credentials.json",
        ".htpasswd",
        ".netrc",
        ".npmrc",
        ".pypirc",
        "app.secrets",
    ];
    for name in &secret_names {
        assert!(is_secret_file(name), "{name} should be detected as secret");
    }
}

#[test]
fn non_secret_files_pass() {
    let safe_names = [
        "main.py",
        "utils.js",
        "README.md",
        "config.yaml",
        "server.go",
        "Dockerfile",
        "package.json",
    ];
    for name in &safe_names {
        assert!(
            !is_secret_file(name),
            "{name} should NOT be detected as secret"
        );
    }
}

#[test]
fn secret_in_subdirectory() {
    assert!(is_secret_file("config/.env"));
    assert!(is_secret_file("deploy/certs/server.pem"));
}

#[test]
fn secret_detection_case_insensitive() {
    assert!(is_secret_file(".ENV"));
    assert!(is_secret_file("Server.PEM"));
}

#[test]
fn doc_files_about_secrets_not_flagged() {
    let doc_paths = [
        "docs/secrets-handling.md",
        "docs/internal/secrets-management.md",
        "guides/secrets-guide.rst",
        "how-to-manage-secrets.txt",
        "security/secret-rotation.adoc",
        "notebooks/secrets-demo.ipynb",
        "docs/secrets.html",
    ];
    for path in &doc_paths {
        assert!(
            !is_secret_file(path),
            "{path} is a doc file and should NOT be flagged as secret"
        );
    }
}

#[test]
fn non_doc_secret_files_still_flagged() {
    let secret_paths = [
        "config/secrets.yaml",
        "config/secrets.json",
        "src/secrets.py",
        ".secrets",
        "app.secrets",
        "my-app-secrets",
    ];
    for path in &secret_paths {
        assert!(is_secret_file(path), "{path} should be flagged as secret");
    }
}

// ── Binary Detection (S-05) ─────────────────────────────────────────────────

#[test]
fn binary_extensions_detected() {
    let binary_exts = [
        ".exe", ".dll", ".so", ".png", ".jpg", ".zip", ".wasm", ".pyc", ".class", ".pdf", ".db",
        ".sqlite",
    ];
    for ext in &binary_exts {
        let filename = format!("file{ext}");
        assert!(
            is_binary_extension(&filename),
            "{filename} should be binary"
        );
    }
}

#[test]
fn source_extensions_not_binary() {
    let source_exts = [".py", ".js", ".ts", ".go", ".rs", ".java", ".md", ".txt"];
    for ext in &source_exts {
        let filename = format!("file{ext}");
        assert!(
            !is_binary_extension(&filename),
            "{filename} should NOT be binary"
        );
    }
}

#[test]
fn binary_content_with_null_bytes() {
    let data = b"Hello\x00World";
    assert!(is_binary_content(data));
}

#[test]
fn text_content_no_null_bytes() {
    let data = b"def hello():\n    print('world')\n";
    assert!(!is_binary_content(data));
}

#[test]
fn empty_content_not_binary() {
    assert!(!is_binary_content(b""));
}

#[test]
fn binary_file_detection_by_extension_and_content() {
    let dir = tempdir().unwrap();

    // Binary by extension
    let bin_file = dir.path().join("image.png");
    fs::write(&bin_file, b"\x89PNG\r\n\x1a\n").unwrap();
    assert!(is_binary_file(&bin_file));

    // Binary by content (null bytes in .py file)
    let sneaky = dir.path().join("data.py");
    fs::write(&sneaky, b"import os\x00\nprint('hi')").unwrap();
    assert!(is_binary_file(&sneaky));

    // Normal text file
    let normal = dir.path().join("main.py");
    fs::write(&normal, "def foo(): pass\n").unwrap();
    assert!(!is_binary_file(&normal));
}

// ── Encoding Safety (S-06) ──────────────────────────────────────────────────

#[test]
fn safe_decode_valid_utf8() {
    let data = "Hello, World!".as_bytes();
    assert_eq!(safe_decode(data), "Hello, World!");
}

#[test]
fn safe_decode_invalid_bytes_replaced_not_crash() {
    let data: &[u8] = b"Hello \xff\xfe World";
    let result = safe_decode(data);
    assert!(result.contains("Hello"));
    assert!(result.contains("World"));
    // Invalid bytes replaced with U+FFFD.
    assert!(result.contains('\u{FFFD}'));
}

#[test]
fn safe_decode_latin1_bytes_does_not_crash() {
    let data: &[u8] = &[0x63, 0x61, 0x66, 0xe9]; // "caf\xe9" in latin-1
    let result = safe_decode(data);
    // Should not panic; produces a String (lossy-replaced).
    assert!(result.starts_with("caf"));
}

// ── File Size Limits ────────────────────────────────────────────────────────

#[test]
fn large_file_size_excluded() {
    let dir = tempdir().unwrap();
    let big_file = dir.path().join("big.py");
    let content = vec![b'x'; 600 * 1024]; // 600KB
    fs::write(&big_file, &content).unwrap();

    let opts = ExcludeOptions {
        max_file_size: 500 * 1024,
        ..Default::default()
    };
    let result = should_exclude_file(&big_file, dir.path(), &opts);
    assert_eq!(
        result.map(|r| r.as_str().to_string()),
        Some("file_too_large".to_string())
    );
}

// ── Composite Filter (should_exclude_file) ──────────────────────────────────

#[test]
fn composite_normal_file_passes() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("main.py");
    fs::write(&f, "def foo(): pass\n").unwrap();

    assert!(should_exclude_file(&f, dir.path(), &ExcludeOptions::default()).is_none());
}

#[test]
fn composite_secret_file_excluded() {
    let dir = tempdir().unwrap();
    let f = dir.path().join(".env");
    fs::write(&f, "SECRET=foo\n").unwrap();

    let result = should_exclude_file(&f, dir.path(), &ExcludeOptions::default());
    assert_eq!(
        result.map(|r| r.as_str().to_string()),
        Some("secret_file".to_string())
    );
}

#[test]
fn composite_large_file_excluded() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("huge.py");
    let content = vec![b'x'; 600 * 1024];
    fs::write(&f, &content).unwrap();

    let result = should_exclude_file(&f, dir.path(), &ExcludeOptions::default());
    assert_eq!(
        result.map(|r| r.as_str().to_string()),
        Some("file_too_large".to_string())
    );
}

#[test]
fn composite_binary_extension_excluded() {
    let dir = tempdir().unwrap();
    let f = dir.path().join("image.png");
    fs::write(&f, b"\x89PNG").unwrap();

    let result = should_exclude_file(&f, dir.path(), &ExcludeOptions::default());
    assert_eq!(
        result.map(|r| r.as_str().to_string()),
        Some("binary_extension".to_string())
    );
}

#[test]
fn composite_secret_check_can_be_disabled() {
    let dir = tempdir().unwrap();
    let f = dir.path().join(".env");
    fs::write(&f, "SECRET=foo\n").unwrap();

    let opts = ExcludeOptions {
        check_secrets: false,
        ..Default::default()
    };
    assert!(should_exclude_file(&f, dir.path(), &opts).is_none());
}

// ── Config: max_index_files ─────────────────────────────────────────────────

#[test]
fn max_index_files_defaults_when_unset() {
    let config = SecurityConfig::default();
    assert_eq!(
        get_max_index_files(&config, None).unwrap(),
        DEFAULT_MAX_INDEX_FILES
    );
}

#[test]
fn max_index_files_reads_config_override() {
    let config = SecurityConfig {
        max_index_files: Some(1234),
        ..Default::default()
    };
    assert_eq!(get_max_index_files(&config, None).unwrap(), 1234);
}

#[test]
fn max_index_files_invalid_config_falls_back_to_default() {
    let config = SecurityConfig {
        max_index_files: Some(0),
        ..Default::default()
    };
    assert_eq!(
        get_max_index_files(&config, None).unwrap(),
        DEFAULT_MAX_INDEX_FILES
    );
}

#[test]
fn max_index_files_non_positive_explicit_rejected() {
    let config = SecurityConfig::default();
    assert!(get_max_index_files(&config, Some(0)).is_err());
}

// ── Config: max_folder_files ────────────────────────────────────────────────

#[test]
fn max_folder_files_default_lower_than_repo_default() {
    assert!(DEFAULT_MAX_FOLDER_FILES < DEFAULT_MAX_INDEX_FILES);
}

#[test]
fn max_folder_files_defaults_when_unset() {
    let config = SecurityConfig::default();
    assert_eq!(
        get_max_folder_files(&config, None).unwrap(),
        DEFAULT_MAX_FOLDER_FILES
    );
}

#[test]
fn max_folder_files_config_takes_priority() {
    let config = SecurityConfig {
        max_folder_files: Some(500),
        ..Default::default()
    };
    assert_eq!(get_max_folder_files(&config, None).unwrap(), 500);
}

#[test]
fn max_folder_files_invalid_falls_back_to_default() {
    let config = SecurityConfig {
        max_folder_files: Some(0),
        ..Default::default()
    };
    // 0 in config is invalid → fall back to default.
    assert_eq!(
        get_max_folder_files(&config, None).unwrap(),
        DEFAULT_MAX_FOLDER_FILES
    );
}

#[test]
fn max_folder_files_explicit_override_respected() {
    let config = SecurityConfig::default();
    assert_eq!(get_max_folder_files(&config, Some(42)).unwrap(), 42);
}

#[test]
fn max_folder_files_non_positive_explicit_rejected() {
    let config = SecurityConfig::default();
    assert!(get_max_folder_files(&config, Some(0)).is_err());
}

// ── Config: extra_ignore_patterns ───────────────────────────────────────────

#[test]
fn extra_ignore_no_config_returns_empty() {
    let config = SecurityConfig::default();
    assert!(get_extra_ignore_patterns(&config, &[]).is_empty());
}

#[test]
fn extra_ignore_call_patterns_only() {
    let config = SecurityConfig::default();
    let result = get_extra_ignore_patterns(&config, &["*.log", "tmp/"]);
    assert_eq!(result, vec!["*.log".to_string(), "tmp/".to_string()]);
}

#[test]
fn extra_ignore_config_patterns_used() {
    let config = SecurityConfig {
        extra_ignore_patterns: vec!["**/scrapes/**".into(), "*.png".into()],
        ..Default::default()
    };
    let result = get_extra_ignore_patterns(&config, &[]);
    assert!(result.contains(&"**/scrapes/**".to_string()));
    assert!(result.contains(&"*.png".to_string()));
}

#[test]
fn extra_ignore_config_and_call_merged() {
    let config = SecurityConfig {
        extra_ignore_patterns: vec!["global/".into()],
        ..Default::default()
    };
    let result = get_extra_ignore_patterns(&config, &["local/"]);
    assert!(result.contains(&"global/".to_string()));
    assert!(result.contains(&"local/".to_string()));
}

#[test]
fn extra_ignore_empty_config_returns_call_only() {
    let config = SecurityConfig::default();
    let result = get_extra_ignore_patterns(&config, &["only/"]);
    assert_eq!(result, vec!["only/".to_string()]);
}

// ── Integration: discover files with security filters ───────────────────────

#[test]
fn discover_excludes_secret_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.py"), "def main(): pass\n").unwrap();
    fs::write(root.join(".env"), "SECRET=foo\n").unwrap();
    fs::write(root.join("config.pem"), "-----BEGIN CERTIFICATE-----\n").unwrap();

    let result = discover_files(&root, &DiscoverOptions::default());

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"main.py".to_string()));
    assert!(!names.contains(&".env".to_string()));
    assert!(!names.contains(&"config.pem".to_string()));

    // Warnings should mention secret.
    let warning_reasons: Vec<&str> = result.warnings.iter().map(|w| w.reason.as_str()).collect();
    assert!(warning_reasons.iter().any(|r| r.contains("secret")));
}

#[test]
fn discover_excludes_binary_files() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("good.py"), "x = 1\n").unwrap();
    fs::write(root.join("bad.py"), b"import os\x00\nprint('hi')").unwrap();

    let result = discover_files(&root, &DiscoverOptions::default());

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"good.py".to_string()));
    assert!(!names.contains(&"bad.py".to_string()));
}

#[test]
fn discover_respects_gitignore() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    // Init a git repo so the ignore crate respects .gitignore.
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&root)
        .output()
        .ok();
    fs::write(root.join(".gitignore"), "ignored.py\n").unwrap();
    fs::write(root.join("kept.py"), "x = 1\n").unwrap();
    fs::write(root.join("ignored.py"), "y = 2\n").unwrap();

    let result = discover_files(&root, &DiscoverOptions::default());

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"kept.py".to_string()));
    assert!(!names.contains(&"ignored.py".to_string()));
}

#[test]
fn discover_extra_ignore_patterns() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.py"), "x = 1\n").unwrap();
    fs::write(root.join("temp.py"), "y = 2\n").unwrap();

    let opts = DiscoverOptions {
        extra_ignore_patterns: vec!["temp.py".into()],
        ..Default::default()
    };
    let result = discover_files(&root, &opts);

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"main.py".to_string()));
    assert!(!names.contains(&"temp.py".to_string()));
}

#[test]
fn discover_respects_config_file_limit() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    for i in 0..10 {
        fs::write(root.join(format!("file{i}.py")), format!("x = {i}\n")).unwrap();
    }

    let opts = DiscoverOptions {
        max_files: Some(3),
        ..Default::default()
    };
    let result = discover_files(&root, &opts);
    assert_eq!(result.files.len(), 3);
}

#[test]
fn discover_symlinks_skipped_by_default() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    let real = root.join("real.py");
    fs::write(&real, "x = 1\n").unwrap();
    let link = root.join("link.py");
    unix_fs::symlink(&real, &link).unwrap();

    let result = discover_files(&root, &DiscoverOptions::default());

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"real.py".to_string()));
    assert!(!names.contains(&"link.py".to_string()));
}

// ── Encoding safety in index store ──────────────────────────────────────────

#[test]
fn read_content_handles_invalid_utf8() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("test.py");
    fs::write(&file, b"def foo():\n    return '\xff\xfe'\n").unwrap();

    // Read and decode — should not panic.
    let raw = fs::read(&file).unwrap();
    let decoded = safe_decode(&raw);
    assert!(decoded.contains("def foo()"));
}

// ── Index repo secret filtering ─────────────────────────────────────────────

#[test]
fn secret_files_filtered_in_remote_discovery() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("proj");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("certs")).unwrap();
    fs::write(root.join("src/main.py"), "def main(): pass\n").unwrap();
    fs::write(root.join("src/utils.py"), "def util(): pass\n").unwrap();
    fs::write(root.join(".env"), "SECRET=foo\n").unwrap();
    fs::write(
        root.join("certs/server.pem"),
        "-----BEGIN CERTIFICATE-----\n",
    )
    .unwrap();

    let result = discover_files(&root, &DiscoverOptions::default());

    let names: Vec<String> = result
        .files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"main.py".to_string()));
    assert!(names.contains(&"utils.py".to_string()));
    assert!(!names.contains(&".env".to_string()));
    assert!(!names.contains(&"server.pem".to_string()));
    assert!(!result.truncated);
}
