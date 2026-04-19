//! Integration tests: extra extensions.

use std::collections::HashMap;
use std::path::Path;

use codesight_extractors::extractors::ExtractorRegistry;

#[test]
fn extra_extensions_suite() {
    run_all();
}

fn run_all() {
    case_registry_resolves_known_extensions();
    case_empty_extra_extensions_no_change();
    case_whitespace_only_extra_extensions_no_change();
}

/// Baseline: the default registry resolves well-known extensions to the correct
/// language extractors.
fn case_registry_resolves_known_extensions() {
    let reg = ExtractorRegistry::default();

    let py = reg.extractor_for_path(Path::new("app.py"));
    assert!(py.is_some(), ".py should resolve");
    assert_eq!(py.unwrap().language_name(), "python");

    let js = reg.extractor_for_path(Path::new("app.js"));
    assert!(js.is_some(), ".js should resolve");
    assert_eq!(js.unwrap().language_name(), "javascript");

    let rs = reg.extractor_for_path(Path::new("lib.rs"));
    assert!(rs.is_some(), ".rs should resolve");
    assert_eq!(rs.unwrap().language_name(), "rust");

    let go = reg.extractor_for_path(Path::new("main.go"));
    assert!(go.is_some(), ".go should resolve");
    assert_eq!(go.unwrap().language_name(), "go");

    let unk = reg.extractor_for_path(Path::new("file.cgi"));
    assert!(unk.is_none(), ".cgi should not resolve by default");
}

fn case_empty_extra_extensions_no_change() {
    let mut reg = ExtractorRegistry::default();
    let warnings = reg.apply_extra_extensions(&HashMap::new());
    assert!(warnings.is_empty());
    let py = reg.extractor_for_path(Path::new("app.py"));
    assert!(py.is_some());
    assert_eq!(py.unwrap().language_name(), "python");
}

fn case_whitespace_only_extra_extensions_no_change() {
    let reg = ExtractorRegistry::default();
    let py = reg.extractor_for_path(Path::new("app.py"));
    assert!(py.is_some());
    assert_eq!(py.unwrap().language_name(), "python");
}

#[test]
fn case_valid_extra_extensions() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".cgi".to_string(), "perl".to_string());
    mappings.insert(".psgi".to_string(), "perl".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        warnings.is_empty(),
        "no warnings expected for valid entries"
    );

    let cgi = reg.extractor_for_path(Path::new("app.cgi"));
    assert!(cgi.is_some(), ".cgi should resolve after extra_extensions");
    assert_eq!(cgi.unwrap().language_name(), "perl");

    let psgi = reg.extractor_for_path(Path::new("app.psgi"));
    assert!(
        psgi.is_some(),
        ".psgi should resolve after extra_extensions"
    );
    assert_eq!(psgi.unwrap().language_name(), "perl");
}

#[test]
fn case_unknown_language_skipped() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".xyz".to_string(), "cobol".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        !warnings.is_empty(),
        "should produce a warning for unknown language"
    );
    assert!(warnings.iter().any(|w| w.contains("cobol")));

    let xyz = reg.extractor_for_path(Path::new("file.xyz"));
    assert!(
        xyz.is_none(),
        ".xyz should not resolve for unknown language"
    );
}

#[test]
fn case_malformed_entry_empty_ext() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert("".to_string(), "perl".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        !warnings.is_empty(),
        "should produce a warning for empty extension"
    );
}

#[test]
fn case_malformed_entry_empty_lang() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".cgi".to_string(), "".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        !warnings.is_empty(),
        "should produce a warning for empty language"
    );

    let cgi = reg.extractor_for_path(Path::new("file.cgi"));
    assert!(
        cgi.is_none(),
        ".cgi should not resolve when language is empty"
    );
}

#[test]
fn case_malformed_entry_no_colon() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".cgiperls".to_string(), "".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        !warnings.is_empty(),
        "should produce a warning for empty language"
    );

    let ext = reg.extractor_for_path(Path::new("file.cgiperls"));
    assert!(ext.is_none(), ".cgiperls should not resolve");
}

#[test]
fn case_override_builtin_extension() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".pl".to_string(), "python".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(warnings.is_empty(), "overriding a builtin should not warn");

    let pl = reg.extractor_for_path(Path::new("script.pl"));
    assert!(pl.is_some(), ".pl should resolve");
    assert_eq!(pl.unwrap().language_name(), "python");
}

#[test]
fn case_mixed_valid_and_invalid() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(".cgi".to_string(), "perl".to_string());
    mappings.insert(".xyz".to_string(), "cobol".to_string());
    mappings.insert(".psgi".to_string(), "perl".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        !warnings.is_empty(),
        "should have at least one warning for cobol"
    );

    let cgi = reg.extractor_for_path(Path::new("app.cgi"));
    assert!(cgi.is_some(), ".cgi should resolve");
    assert_eq!(cgi.unwrap().language_name(), "perl");

    let psgi = reg.extractor_for_path(Path::new("app.psgi"));
    assert!(psgi.is_some(), ".psgi should resolve");
    assert_eq!(psgi.unwrap().language_name(), "perl");

    let xyz = reg.extractor_for_path(Path::new("file.xyz"));
    assert!(xyz.is_none(), ".xyz should not resolve (cobol is unknown)");
}

#[test]
fn case_extra_whitespace_in_entries() {
    let mut reg = ExtractorRegistry::default();
    let mut mappings = HashMap::new();
    mappings.insert(" .cgi ".to_string(), " perl ".to_string());
    mappings.insert(" .psgi ".to_string(), " perl ".to_string());

    let warnings = reg.apply_extra_extensions(&mappings);
    assert!(
        warnings.is_empty(),
        "whitespace-padded entries should still work"
    );

    let cgi = reg.extractor_for_path(Path::new("app.cgi"));
    assert!(cgi.is_some(), ".cgi should resolve after trimming");
    assert_eq!(cgi.unwrap().language_name(), "perl");

    let psgi = reg.extractor_for_path(Path::new("app.psgi"));
    assert!(psgi.is_some(), ".psgi should resolve after trimming");
    assert_eq!(psgi.unwrap().language_name(), "perl");
}
