//! Adaptive language detection (`languages_adaptive` in `.codesight.toml`).

use std::collections::HashSet;

use codesight_mcp::adaptive_languages::{
    CodesightProjectConfig, PROJECT_CONFIG_FILE, apply_adaptive_languages, build_languages_block,
    check_raw_local_adaptive, config_cache_contains, invalidate_project_config_cache,
    parse_active_languages, prime_config_cache,
};
use codesight_mcp::extractors::ExtractorRegistry;

/// Helper: get sorted list of all language names from the registry.
fn all_languages() -> Vec<&'static str> {
    let reg = ExtractorRegistry::default();
    let langs = reg.known_languages();
    let leaked: Vec<&'static str> = langs
        .into_iter()
        .map(|s| &*Box::leak(s.to_string().into_boxed_str()))
        .collect();
    leaked
}

// ---------------------------------------------------------------------------
// TestParseActiveLanguages (5 tests)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_active_languages_normal_array_all_active() {
    let content = r#"languages = ["python", "javascript"]"#;
    let result = parse_active_languages(content);
    assert_eq!(
        result,
        Some(HashSet::from([
            "python".to_string(),
            "javascript".to_string()
        ]))
    );
}

#[test]
fn test_parse_active_languages_skips_disabled_not_listed() {
    let content = r#"# javascript disabled for this workspace
languages = ["python", "rust"]
"#;
    let result = parse_active_languages(content);
    assert_eq!(
        result,
        Some(HashSet::from(["python".to_string(), "rust".to_string()]))
    );
}

#[test]
fn test_parse_active_languages_missing_languages_key() {
    let content = r#"languages_adaptive = true"#;
    let result = parse_active_languages(content);
    assert!(result.is_none());
}

#[test]
fn test_parse_active_languages_absent_languages_key_with_other_fields() {
    let content = r#"other = true"#;
    let result = parse_active_languages(content);
    assert!(result.is_none());
}

#[test]
fn test_parse_active_languages_empty_array() {
    let content = r#"languages = []"#;
    let result = parse_active_languages(content);
    assert_eq!(result, Some(HashSet::new()));
}

// ---------------------------------------------------------------------------
// TestBuildLanguagesBlock (2 tests)
// ---------------------------------------------------------------------------

#[test]
fn test_build_languages_block_detected_subset() {
    let all = all_languages();
    let detected: HashSet<String> = ["python", "javascript"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let block = build_languages_block(&detected, &all);
    let wrapped = format!("languages_adaptive = true\n{block}");
    let cfg: CodesightProjectConfig = toml::from_str(&wrapped).unwrap();
    let got: HashSet<String> = cfg.languages.unwrap_or_default().into_iter().collect();
    assert_eq!(got, detected);

    for lang in &all {
        if !detected.contains(*lang) {
            assert!(
                !got.contains(*lang),
                "language '{lang}' should not be listed when not detected"
            );
        }
    }
}

#[test]
fn test_build_languages_block_empty_detected() {
    let all = all_languages();
    let detected: HashSet<String> = HashSet::new();

    let block = build_languages_block(&detected, &all);
    let wrapped = format!("languages_adaptive = true\n{block}");
    let cfg: CodesightProjectConfig = toml::from_str(&wrapped).unwrap();
    assert_eq!(cfg.languages.unwrap_or_default(), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// TestInvalidateProjectConfigCache (1 test)
// ---------------------------------------------------------------------------

#[test]
fn test_invalidate_project_config_cache_evicts_from_cache() {
    let test_path = "/fake/project/root/for/adaptive/test";

    prime_config_cache(
        test_path,
        serde_json::json!({"languages": ["python"]}),
        "abc123",
    );

    let (in_configs, in_hashes) = config_cache_contains(test_path);
    assert!(in_configs, "config should be cached");
    assert!(in_hashes, "hash should be cached");

    invalidate_project_config_cache(test_path);

    let (in_configs, in_hashes) = config_cache_contains(test_path);
    assert!(!in_configs, "config should be evicted");
    assert!(!in_hashes, "hash should be evicted");
}

// ---------------------------------------------------------------------------
// TestCheckRawLocalAdaptive (4 tests)
// ---------------------------------------------------------------------------

#[test]
fn test_check_raw_local_adaptive_true_in_local_config() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    let content = "languages_adaptive = true\nlanguages = [\"python\"]\n";
    std::fs::write(&cfg_path, content).unwrap();

    let (is_adaptive, raw) = check_raw_local_adaptive(&cfg_path);
    assert!(is_adaptive);
    assert_eq!(raw, content);
}

#[test]
fn test_check_raw_local_adaptive_false_in_local_config() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    std::fs::write(&cfg_path, "languages_adaptive = false\n").unwrap();

    let (is_adaptive, _) = check_raw_local_adaptive(&cfg_path);
    assert!(!is_adaptive);
}

#[test]
fn test_check_raw_local_adaptive_absent_key() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    std::fs::write(&cfg_path, "languages = [\"python\"]\n").unwrap();

    let (is_adaptive, _) = check_raw_local_adaptive(&cfg_path);
    assert!(!is_adaptive);
}

#[test]
fn test_check_raw_local_adaptive_invalid_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    std::fs::write(&cfg_path, "not valid toml [[[\n").unwrap();

    let (is_adaptive, content) = check_raw_local_adaptive(&cfg_path);
    assert!(!is_adaptive);
    assert_eq!(content, "");
}

// ---------------------------------------------------------------------------
// TestApplyAdaptiveLanguages (4 tests)
// ---------------------------------------------------------------------------

#[test]
fn test_apply_adaptive_languages_no_local_global_adaptive_creates_file() {
    let tmp = tempfile::tempdir().unwrap();
    let all = all_languages();
    let detected: HashSet<String> = ["python", "rust"].iter().map(|s| s.to_string()).collect();

    let _result = apply_adaptive_languages(tmp.path().to_str().unwrap(), &detected, true, &all);

    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    assert!(cfg_path.exists(), "config file should be created");

    let content = std::fs::read_to_string(&cfg_path).unwrap();
    let parsed: CodesightProjectConfig = toml::from_str(&content).unwrap();
    assert!(parsed.languages_adaptive);
}

#[test]
fn test_apply_adaptive_languages_local_exists_adaptive_true_updates_languages() {
    let tmp = tempfile::tempdir().unwrap();
    let all = all_languages();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    std::fs::write(
        &cfg_path,
        "languages_adaptive = true\nlanguages = [\"python\"]\n",
    )
    .unwrap();

    let detected: HashSet<String> = ["python", "javascript", "rust"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let result = apply_adaptive_languages(tmp.path().to_str().unwrap(), &detected, false, &all);

    assert!(result, "should return true when languages updated");
    let content = std::fs::read_to_string(&cfg_path).unwrap();
    for lang in &detected {
        assert!(
            content.contains(&format!("\"{lang}\"")),
            "language '{lang}' should appear in updated file"
        );
    }
}

#[test]
fn test_apply_adaptive_languages_noop_when_adaptive_false() {
    let tmp = tempfile::tempdir().unwrap();
    let all = all_languages();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    let original = "languages_adaptive = false\nlanguages = [\"python\"]\n";
    std::fs::write(&cfg_path, original).unwrap();

    let detected: HashSet<String> = ["python", "rust"].iter().map(|s| s.to_string()).collect();

    let result = apply_adaptive_languages(tmp.path().to_str().unwrap(), &detected, false, &all);

    assert!(!result, "should return false when adaptive is false");
    let content = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(content, original, "file should be unchanged");
}

#[test]
fn test_apply_adaptive_languages_noop_when_no_change_needed() {
    let tmp = tempfile::tempdir().unwrap();
    let all = all_languages();
    let cfg_path = tmp.path().join(PROJECT_CONFIG_FILE);
    let original = "languages_adaptive = true\nlanguages = [\"python\"]\n";
    std::fs::write(&cfg_path, original).unwrap();

    let detected: HashSet<String> = ["python"].iter().map(|s| s.to_string()).collect();

    let result = apply_adaptive_languages(tmp.path().to_str().unwrap(), &detected, true, &all);

    assert!(!result, "should return false when no change needed");
    let content = std::fs::read_to_string(&cfg_path).unwrap();
    assert_eq!(content, original, "file should be unchanged");
}
