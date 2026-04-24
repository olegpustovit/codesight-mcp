//! Integration tests: extra extensions.

use std::collections::HashMap;
use std::path::Path;

use codesight_language_support::LanguageRegistry;

fn lang_at(reg: &LanguageRegistry, path: &str) -> Option<&'static str> {
    reg.for_path(Path::new(path)).map(|lang| lang.id().as_str())
}

#[test]
fn extra_extensions_suite() {
    case_registry_resolves_known_extensions();
    case_empty_extra_extensions_no_change();
    case_whitespace_only_extra_extensions_no_change();
}

fn case_registry_resolves_known_extensions() {
    let reg = LanguageRegistry::default();
    assert_eq!(lang_at(&reg, "app.py"), Some("python"));
    assert_eq!(lang_at(&reg, "app.js"), Some("javascript"));
    assert_eq!(lang_at(&reg, "lib.rs"), Some("rust"));
    assert_eq!(lang_at(&reg, "main.go"), Some("go"));
    // `.cgi` is a Perl extension per the builtin manifest.
    assert_eq!(lang_at(&reg, "file.cgi"), Some("perl"));
}

fn case_empty_extra_extensions_no_change() {
    let mut reg = LanguageRegistry::default();
    let warnings = reg.apply_extra_extensions(&HashMap::new());
    assert!(warnings.is_empty());
    assert_eq!(lang_at(&reg, "app.py"), Some("python"));
}

fn case_whitespace_only_extra_extensions_no_change() {
    let reg = LanguageRegistry::default();
    assert_eq!(lang_at(&reg, "app.py"), Some("python"));
}

#[test]
fn case_valid_extra_extensions() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".xperl".to_string(), "perl".to_string());
    mappings.insert(".yperl".to_string(), "perl".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        warnings.is_empty(),
        "no warnings expected for valid entries"
    );
    assert_eq!(lang_at(&reg, "app.xperl"), Some("perl"));
    assert_eq!(lang_at(&reg, "app.yperl"), Some("perl"));
}

#[test]
fn case_unknown_language_skipped() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".xyz".to_string(), "cobol".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(warnings.iter().any(|w| w.contains("cobol")));
    assert!(lang_at(&reg, "file.xyz").is_none());
}

#[test]
fn case_malformed_entry_empty_ext() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert("".to_string(), "perl".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(!warnings.is_empty());
}

#[test]
fn case_malformed_entry_empty_lang() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".xyz".to_string(), "".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(!warnings.is_empty());
    assert!(lang_at(&reg, "file.xyz").is_none());
}

#[test]
fn case_override_builtin_extension() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".pl".to_string(), "python".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(warnings.is_empty(), "overriding a builtin should not warn");
    assert_eq!(lang_at(&reg, "script.pl"), Some("python"));
}

#[test]
fn case_mixed_valid_and_invalid() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".xperl".to_string(), "perl".to_string());
    mappings.insert(".xyz".to_string(), "cobol".to_string());
    mappings.insert(".yperl".to_string(), "perl".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(!warnings.is_empty());
    assert_eq!(lang_at(&reg, "app.xperl"), Some("perl"));
    assert_eq!(lang_at(&reg, "app.yperl"), Some("perl"));
    assert!(lang_at(&reg, "file.xyz").is_none());
}

#[test]
fn case_extra_whitespace_in_entries() {
    let mut reg = LanguageRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(" .xperl ".to_string(), " perl ".to_string());
    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(warnings.is_empty());
    assert_eq!(lang_at(&reg, "app.xperl"), Some("perl"));
}
