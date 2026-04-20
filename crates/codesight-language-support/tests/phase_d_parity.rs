//! Phase D parity tests (LANGUAGES.md §7.2): liveness, visibility, and
//! complexity heuristics behave byte-for-byte like the old
//! `codesight-analysis` scattered implementations.

use codesight_language_support::{
    fallback_type_baseline_for_path, has_restricted_visibility, is_conventional_path,
    is_entrypoint_basename, is_test_file, language_family_for_path,
    max_nesting_depth_for_path, measured_type_density_for_path, test_weight_multiplier,
};
use codesight_language_support::traits::complexity::LanguageFamily;

// ── §4.4 liveness ───────────────────────────────────────────────────────────

#[test]
fn every_curated_language_entrypoint_basename_is_discoverable() {
    for basename in [
        "main.rs",
        "lib.rs",
        "build.rs",
        "main.py",
        "__main__.py",
        "__init__.py",
        "app.py",
        "setup.py",
        "conftest.py",
        "main.go",
        "index.ts",
        "server.ts",
        "index.js",
    ] {
        assert!(
            is_entrypoint_basename(basename),
            "{basename} should register as an entrypoint"
        );
    }
}

#[test]
fn random_file_is_not_an_entrypoint() {
    assert!(!is_entrypoint_basename("random.rs"));
    assert!(!is_entrypoint_basename("mod.rs"));
    assert!(!is_entrypoint_basename("helpers.py"));
}

#[test]
fn rust_tests_directory_is_test_file() {
    assert!(is_test_file("tests/integration.rs"));
    assert!(is_test_file("crates/foo/tests/it.rs"));
    assert!(is_test_file("src/foo_test.rs"));
}

#[test]
fn javascript_test_suffixes_are_test_files() {
    assert!(is_test_file("src/foo.test.ts"));
    assert!(is_test_file("src/foo.test.tsx"));
    assert!(is_test_file("src/foo.test.js"));
    assert!(is_test_file("src/foo_test.js"));
}

#[test]
fn ruby_spec_files_are_test_files() {
    assert!(is_test_file("spec/foo_spec.rb"));
    assert!(is_test_file("src/foo_spec.rb"));
}

#[test]
fn python_test_files_detected_by_prefix_and_suffix() {
    assert!(is_test_file("tests/test_auth.py"));
    assert!(is_test_file("tests/test_auth.py"));
    assert!(is_test_file("src/auth_test.py"));
}

#[test]
fn production_code_is_not_test_file() {
    assert!(!is_test_file("src/lib.rs"));
    assert!(!is_test_file("crates/foo/src/mod.rs"));
    assert!(!is_test_file("app/views/user.py"));
}

#[test]
fn conventional_paths_match_scripts_tests_examples_benches() {
    assert!(is_conventional_path("scripts/generate.py"));
    assert!(is_conventional_path("tests/foo.rs"));
    assert!(is_conventional_path("examples/demo.rs"));
    assert!(is_conventional_path("benches/bench_a.rs"));
    assert!(is_conventional_path("fixtures/sample.json"));
    assert!(is_conventional_path("build.rs"));
    assert!(is_conventional_path("a/b/conftest.py"));
    assert!(is_conventional_path("crates/x/tests/it.rs"));
}

#[test]
fn conventional_path_lets_production_code_through() {
    assert!(!is_conventional_path("src/main.rs"));
    assert!(!is_conventional_path("crates/codesight-engine/src/service/mod.rs"));
    assert!(!is_conventional_path("app/views/user.py"));
}

#[test]
fn test_weight_multiplier_downweights_tests_and_specs() {
    assert!((test_weight_multiplier("tests/foo.rs") - 0.3).abs() < 1e-9);
    assert!((test_weight_multiplier("crates/x/tests/foo.rs") - 0.3).abs() < 1e-9);
    assert!((test_weight_multiplier("src/foo_test.py") - 0.3).abs() < 1e-9);
    assert!((test_weight_multiplier("src/foo.test.ts") - 0.3).abs() < 1e-9);
    assert!((test_weight_multiplier("src/foo_spec.rb") - 0.3).abs() < 1e-9);
    assert!((test_weight_multiplier("src/main.rs") - 1.0).abs() < 1e-9);
}

// ── §4.5 visibility ────────────────────────────────────────────────────────

#[test]
fn rust_non_pub_is_restricted() {
    assert!(has_restricted_visibility("src/lib.rs", "fn foo() {}", "foo"));
    assert!(has_restricted_visibility(
        "src/lib.rs",
        "pub(crate) fn foo() {}",
        "foo"
    ));
    assert!(!has_restricted_visibility(
        "src/lib.rs",
        "pub fn foo() {}",
        "foo"
    ));
}

#[test]
fn go_lowercase_is_restricted() {
    assert!(has_restricted_visibility("main.go", "func foo() {}", "foo"));
    assert!(!has_restricted_visibility("main.go", "func Foo() {}", "Foo"));
}

#[test]
fn python_underscore_is_restricted() {
    assert!(has_restricted_visibility("a.py", "def _foo(): ...", "_foo"));
    assert!(!has_restricted_visibility("a.py", "def foo(): ...", "foo"));
}

#[test]
fn typescript_private_is_restricted() {
    assert!(has_restricted_visibility(
        "a.ts",
        "private foo() {}",
        "foo"
    ));
    assert!(!has_restricted_visibility(
        "a.ts",
        "export function foo() {}",
        "foo"
    ));
}

#[test]
fn java_public_marker_is_required() {
    assert!(has_restricted_visibility(
        "Foo.java",
        "private void foo() {}",
        "foo"
    ));
    assert!(!has_restricted_visibility(
        "Foo.java",
        "public void foo() {}",
        "foo"
    ));
}

// ── §4.6 complexity / type density ─────────────────────────────────────────

#[test]
fn rust_family_is_braced_static() {
    assert_eq!(
        language_family_for_path("src/lib.rs"),
        LanguageFamily::BracedStatic
    );
}

#[test]
fn python_family_is_scripted_dynamic() {
    assert_eq!(
        language_family_for_path("src/lib.py"),
        LanguageFamily::ScriptedDynamic
    );
}

#[test]
fn elixir_family_is_functional() {
    assert_eq!(
        language_family_for_path("lib/foo.ex"),
        LanguageFamily::FunctionalExpression
    );
}

#[test]
fn yaml_family_is_declarative() {
    assert_eq!(
        language_family_for_path("openapi.yaml"),
        LanguageFamily::DeclarativeMarkup
    );
}

#[test]
fn unknown_extension_family_is_unknown() {
    assert_eq!(
        language_family_for_path("readme.unknown"),
        LanguageFamily::Unknown
    );
}

#[test]
fn rust_measured_type_density_parses_signatures() {
    let source = "fn run(&self, value: i32) {}";
    let score = measured_type_density_for_path("lib.rs", source).unwrap();
    assert!((score - 1.0).abs() < 0.01);
}

#[test]
fn python_fallback_baseline_is_point_four() {
    assert_eq!(fallback_type_baseline_for_path("src/lib.py"), Some(0.4));
}

#[test]
fn typescript_fallback_baseline_is_point_seven() {
    assert_eq!(fallback_type_baseline_for_path("src/lib.ts"), Some(0.7));
}

#[test]
fn javascript_fallback_baseline_is_point_one() {
    assert_eq!(fallback_type_baseline_for_path("src/app.js"), Some(0.1));
}

#[test]
fn rust_brace_nesting_counts_opening_braces() {
    let source = "fn foo(x: i32) {\n    match x {\n        1 => println!(\"one\"),\n        _ => println!(\"other\"),\n    }\n}";
    assert_eq!(max_nesting_depth_for_path(source, "lib.rs"), 2);
}

#[test]
fn python_indent_nesting_tracks_blocks() {
    let source = "def foo(x):\n    if x:\n        for item in x:\n            return item\n";
    assert_eq!(max_nesting_depth_for_path(source, "lib.py"), 3);
}
