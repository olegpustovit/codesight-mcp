//! Integration tests: css.

use std::collections::HashSet;
use std::path::Path;

use codesight_extractors::extractors::engine::scm::{QUERY_FILES, ScmExtractor, parse_scm_header};
use codesight_extractors::extractors::{
    ExtractorRegistry, LanguageExtractor, LessExtractor, SassExtractor, StylExtractor,
};
use codesight_core::SymbolKind;

fn scm_extractor(lang_key: &str) -> ScmExtractor {
    let source = QUERY_FILES
        .iter()
        .find(|(k, _)| *k == lang_key)
        .unwrap_or_else(|| panic!("no query for {lang_key}"))
        .1;
    let meta = parse_scm_header(source);
    ScmExtractor::new(&meta, source).unwrap()
}

// ---------------------------------------------------------------------------
// Inline fixtures (matching reference _CSS_SRC / _SCSS_SRC)
// ---------------------------------------------------------------------------

const CSS_SRC: &str = r#"
:root {
  --primary-color: #333;
  --spacing: 8px;
}

body {
  margin: 0;
  padding: 0;
  font-family: sans-serif;
}

.container {
  display: flex;
  max-width: 1200px;
}

.navbar .item {
  color: red;
}

#header {
  font-size: 24px;
  font-weight: bold;
}

h1, h2, h3 {
  font-weight: bold;
}

@keyframes slideIn {
  from { transform: translateX(-100%); }
  to   { transform: translateX(0); }
}

@media (max-width: 768px) {
  .container { flex-direction: column; }
}

@supports (display: grid) {
  .layout { display: grid; }
}
"#;

const SCSS_SRC: &str = r#"
$primary-color: #333;
$font-size: 16px;

@mixin flex-center($dir: row) {
  display: flex;
  flex-direction: $dir;
}

@mixin respond-to($breakpoint) {
  @media (max-width: $breakpoint) { @content; }
}

%visually-hidden {
  position: absolute;
  clip: rect(0, 0, 0, 0);
}

.container {
  max-width: 1200px;
}

.navbar .item {
  color: red;
}

@function px-to-rem($px) {
  @return $px / 16 * 1rem;
}

@media (max-width: 768px) {
  .container { flex-direction: column; }
}

@supports (display: grid) {
  .layout { display: grid; }
}
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn css_syms() -> Vec<codesight_core::StoredSymbol> {
    scm_extractor("css")
        .extract("styles/main.css", CSS_SRC)
        .expect("CSS extraction should not fail")
}

fn scss_extractor() -> ScmExtractor {
    scm_extractor("scss")
}

fn scss_syms() -> Vec<codesight_core::StoredSymbol> {
    scss_extractor()
        .extract("styles/main.scss", SCSS_SRC)
        .expect("SCSS extraction should not fail")
}

// ---------------------------------------------------------------------------
// Extension / language detection  (ports test_css_extension_detected, etc.)
// ---------------------------------------------------------------------------

#[test]
fn test_css_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("static/css/main.css"));
    assert!(ext.is_some(), "CSS file should be detected");
    assert_eq!(ext.unwrap().language_name(), "css");
}

#[test]
fn test_css_extension_in_registry() {
    // SCM CSS extractor must list "css" in its supported extensions.
    let ext = scm_extractor("css");
    assert!(
        ext.supported_extensions().contains(&"css"),
        "CSS SCM extractor must support .css"
    );
}

// ---------------------------------------------------------------------------
// CSS symbol extraction
// ---------------------------------------------------------------------------

/// Ref: test_css_returns_symbols — at least 7 symbols from the fixture.
#[test]
fn test_css_returns_symbols() {
    let syms = css_syms();
    assert!(
        syms.len() >= 7,
        "Expected >= 7 CSS symbols, got {}",
        syms.len()
    );
}

/// Ref: test_css_rule_sets_extracted
#[test]
fn test_css_rule_sets_extracted() {
    let syms = css_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(":root"), "missing :root");
    assert!(names.contains("body"), "missing body");
    assert!(names.contains(".container"), "missing .container");
}

/// Ref: test_css_class_selector
#[test]
fn test_css_class_selector() {
    let syms = css_syms();
    let s = syms
        .iter()
        .find(|s| s.name == ".container")
        .expect(".container symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
    assert_eq!(s.language, "css");
    assert!(s.start_line > 0);
}

/// Ref: test_css_id_selector
#[test]
fn test_css_id_selector() {
    let syms = css_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "#header")
        .expect("#header symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
}

/// Ref: test_css_tag_selector
#[test]
fn test_css_tag_selector() {
    let syms = css_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "body")
        .expect("body symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
}

/// Ref: test_css_compound_selector
#[test]
fn test_css_compound_selector() {
    let syms = css_syms();
    let s = syms
        .iter()
        .find(|s| s.name == ".navbar .item")
        .expect(".navbar .item symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
}

/// Ref: test_css_grouped_selectors
#[test]
fn test_css_grouped_selectors() {
    let syms = css_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "h1, h2, h3")
        .expect("h1, h2, h3 grouped selector symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
}

/// Ref: test_css_keyframes_extracted
#[test]
fn test_css_keyframes_extracted() {
    let syms = css_syms();
    let kf = syms
        .iter()
        .find(|s| s.name.contains("@keyframes"))
        .expect("@keyframes symbol missing");
    assert_eq!(kf.name, "@keyframes slideIn");
    assert_eq!(kf.kind, SymbolKind::Function);
}

/// Ref: test_css_media_query_extracted
#[test]
fn test_css_media_query_extracted() {
    let syms = css_syms();
    let mq = syms
        .iter()
        .find(|s| s.name.contains("@media"))
        .expect("@media symbol missing");
    assert!(mq.name.contains("@media"));
    assert_eq!(mq.kind, SymbolKind::Type);
}

/// Ref: test_css_supports_extracted
#[test]
fn test_css_supports_extracted() {
    let syms = css_syms();
    let sup = syms
        .iter()
        .find(|s| s.name.contains("@supports"))
        .expect("@supports symbol missing");
    assert_eq!(sup.kind, SymbolKind::Type);
}

/// Ref: test_css_symbol_ids_unique
#[test]
fn test_css_symbol_ids_unique() {
    let syms = css_syms();
    let ids: Vec<&str> = syms.iter().map(|s| s.id.as_str()).collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "Duplicate symbol IDs found");
}

/// Ref: test_css_symbol_has_byte_info
#[test]
fn test_css_symbol_has_byte_info() {
    let syms = css_syms();
    assert!(!syms.is_empty(), "Need symbols to check byte info");
    for s in &syms {
        // byte_offset >= 0 is trivially true for usize; check start_byte is plausible
        assert!(
            s.end_byte > s.start_byte,
            "byte_length must be > 0 for symbol {}",
            s.name
        );
        assert!(
            !s.source_hash.is_empty(),
            "content_hash must not be empty for symbol {}",
            s.name
        );
    }
}

/// Ref: test_css_via_parse_file — exercise the ExtractorRegistry dispatch path.
#[test]
fn test_css_via_parse_file() {
    let reg = ExtractorRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("styles/main.css"))
        .expect("registry should find CSS extractor");
    let syms = ext
        .extract("styles/main.css", CSS_SRC)
        .expect("extract failed");
    // SCM query extracts selectors + keyframes; fewer than the old CssExtractor's 7+.
    assert!(
        !syms.is_empty(),
        "Expected CSS symbols via parse_file dispatch, got 0"
    );
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(".container"),
        "missing .container via parse_file"
    );
    // SCM captures @keyframes prefix via name_prefix
    assert!(
        names.contains("@keyframes slideIn"),
        "missing @keyframes slideIn via parse_file; got: {:?}",
        names
    );
}

/// Ref: test_css_empty_file
#[test]
fn test_css_empty_file() {
    let empty = scm_extractor("css")
        .extract("empty.css", "")
        .expect("extract empty");
    assert!(empty.is_empty(), "empty CSS should yield no symbols");

    let blank = scm_extractor("css")
        .extract("blank.css", "   \n\n   ")
        .expect("extract blank");
    assert!(blank.is_empty(), "blank CSS should yield no symbols");
}

/// Ref: test_css_no_symbols_file
#[test]
fn test_css_no_symbols_file() {
    let src = "/* just a comment */\n/* another */\n";
    let syms = scm_extractor("css")
        .extract("comments.css", src)
        .expect("extract comments-only");
    assert!(
        syms.is_empty(),
        "CSS with only comments should yield no symbols"
    );
}

// ---------------------------------------------------------------------------
// SCSS extension / language detection
// ---------------------------------------------------------------------------

/// Ref: test_scss_extension_detected
#[test]
fn test_scss_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("styles/main.scss"));
    assert!(ext.is_some(), "SCSS file should be detected");
    assert_eq!(ext.unwrap().language_name(), "scss");
}

/// Ref: test_sass_extension_detected
#[test]
fn test_sass_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("styles/main.sass"));
    assert!(ext.is_some(), "Sass file should be detected");
    assert_eq!(ext.unwrap().language_name(), "sass");
}

/// Ref: test_less_extension_detected
#[test]
fn test_less_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("styles/main.less"));
    assert!(ext.is_some(), "Less file should be detected");
    assert_eq!(ext.unwrap().language_name(), "less");
}

/// Ref: test_styl_extension_detected
#[test]
fn test_styl_extension_detected() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("styles/main.styl"));
    assert!(ext.is_some(), "Styl file should be detected");
    assert_eq!(ext.unwrap().language_name(), "styl");
}

/// Ref: test_scss_extension_in_registry
#[test]
fn test_scss_extension_in_registry() {
    assert!(
        scss_extractor().supported_extensions().contains(&"scss"),
        "SCSS query pack must support .scss"
    );
}

/// Ref: test_less_extension_in_registry
#[test]
fn test_less_extension_in_registry() {
    assert!(
        LessExtractor.supported_extensions().contains(&"less"),
        "LessExtractor must support .less"
    );
}

// ---------------------------------------------------------------------------
// SCSS symbol extraction
// ---------------------------------------------------------------------------

/// Ref: test_scss_returns_symbols
#[test]
fn test_scss_returns_symbols() {
    let syms = scss_syms();
    assert!(
        syms.len() >= 6,
        "Expected >= 6 SCSS symbols, got {}",
        syms.len()
    );
}

/// Ref: test_scss_variables_extracted
#[test]
fn test_scss_variables_extracted() {
    let syms = scss_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains("$primary-color"), "missing $primary-color");
    assert!(names.contains("$font-size"), "missing $font-size");
}

/// Ref: test_scss_variable_kind
#[test]
fn test_scss_variable_kind() {
    let syms = scss_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "$primary-color")
        .expect("$primary-color symbol missing");
    assert_eq!(s.kind, SymbolKind::Constant);
    assert_eq!(s.language, "scss");
    assert!(
        s.signature.contains("$primary-color"),
        "signature should contain $primary-color, got: {}",
        s.signature
    );
}

/// Ref: test_scss_mixin_extracted
#[test]
fn test_scss_mixin_extracted() {
    let syms = scss_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains("@mixin flex-center"),
        "missing @mixin flex-center"
    );
}

/// Ref: test_scss_mixin_kind
#[test]
fn test_scss_mixin_kind() {
    let syms = scss_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "@mixin flex-center")
        .expect("@mixin flex-center symbol missing");
    assert_eq!(s.kind, SymbolKind::Function);
    assert!(
        s.signature.contains("flex-center"),
        "mixin signature should contain flex-center, got: {}",
        s.signature
    );
}

/// Ref: test_scss_function_extracted
#[test]
fn test_scss_function_extracted() {
    let syms = scss_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains("@function px-to-rem"),
        "missing @function px-to-rem"
    );
}

/// Ref: test_scss_function_kind
#[test]
fn test_scss_function_kind() {
    let syms = scss_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "@function px-to-rem")
        .expect("@function px-to-rem symbol missing");
    assert_eq!(s.kind, SymbolKind::Function);
}

/// Ref: test_scss_rule_set_extracted
#[test]
fn test_scss_rule_set_extracted() {
    let syms = scss_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(".container"), "missing .container");
}

/// Ref: test_scss_rule_set_kind
#[test]
fn test_scss_rule_set_kind() {
    let syms = scss_syms();
    let s = syms
        .iter()
        .find(|s| s.name == ".container")
        .expect(".container symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
    assert_eq!(s.language, "scss");
}

/// Ref: test_scss_placeholder_extracted
#[test]
fn test_scss_placeholder_extracted() {
    let syms = scss_syms();
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains("%visually-hidden"),
        "missing %visually-hidden"
    );
}

/// Ref: test_scss_placeholder_kind
#[test]
fn test_scss_placeholder_kind() {
    let syms = scss_syms();
    let s = syms
        .iter()
        .find(|s| s.name == "%visually-hidden")
        .expect("%visually-hidden symbol missing");
    assert_eq!(s.kind, SymbolKind::Class);
}

/// Ref: test_scss_media_query_extracted
#[test]
fn test_scss_media_query_extracted() {
    let syms = scss_syms();
    let mq = syms.iter().find(|s| s.name.contains("@media"));
    assert!(mq.is_some(), "@media symbol missing in SCSS");
    assert_eq!(mq.unwrap().kind, SymbolKind::Type);
}

/// Ref: test_scss_supports_extracted
#[test]
fn test_scss_supports_extracted() {
    let syms = scss_syms();
    let sup = syms.iter().find(|s| s.name.contains("@supports"));
    assert!(sup.is_some(), "@supports symbol missing in SCSS");
    assert_eq!(sup.unwrap().kind, SymbolKind::Type);
}

/// Ref: test_scss_symbol_ids_unique
#[test]
fn test_scss_symbol_ids_unique() {
    let syms = scss_syms();
    let mut seen = HashSet::new();
    for s in &syms {
        assert!(
            seen.insert(s.id.as_str()),
            "Duplicate SCSS symbol ID: {}",
            s.id
        );
    }
}

/// Ref: test_scss_symbol_has_byte_info
#[test]
fn test_scss_symbol_has_byte_info() {
    let syms = scss_syms();
    assert!(!syms.is_empty(), "Need SCSS symbols to check byte info");
    for s in &syms {
        assert!(
            s.end_byte > s.start_byte,
            "byte_length must be > 0 for SCSS symbol {}",
            s.name
        );
        assert!(
            !s.source_hash.is_empty(),
            "content_hash must not be empty for SCSS symbol {}",
            s.name
        );
    }
}

/// Ref: test_scss_via_parse_file — exercise ExtractorRegistry dispatch for SCSS.
#[test]
fn test_scss_via_parse_file() {
    let reg = ExtractorRegistry::default();
    let ext = reg
        .extractor_for_path(Path::new("styles/main.scss"))
        .expect("registry should find SCSS extractor");
    let syms = ext
        .extract("styles/main.scss", SCSS_SRC)
        .expect("extract failed");
    assert!(
        syms.len() >= 6,
        "Expected >= 6 SCSS symbols via parse_file dispatch, got {}",
        syms.len()
    );
    let names: HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains("$primary-color"),
        "missing $primary-color via parse_file"
    );
    assert!(
        names.contains("@mixin flex-center"),
        "missing @mixin flex-center via parse_file"
    );
}

/// Ref: test_scss_empty_file
#[test]
fn test_scss_empty_file() {
    let syms = scss_extractor()
        .extract("empty.scss", "")
        .expect("extract empty SCSS");
    assert!(syms.is_empty(), "empty SCSS should yield no symbols");
}

// ---------------------------------------------------------------------------
// Less / Sass / Styl — text-search only (no symbol extraction)
// ---------------------------------------------------------------------------

/// Ref: test_less_parse_returns_empty_symbols
#[test]
fn test_less_parse_returns_empty_symbols() {
    let src = "@primary: #333;\n.container { color: @primary; }\n";
    let syms = LessExtractor
        .extract("styles/main.less", src)
        .expect("Less extract");
    assert!(
        syms.is_empty(),
        "Less has no grammar; parse should return empty symbols"
    );
}

/// Ref: test_sass_parse_returns_empty_symbols
#[test]
fn test_sass_parse_returns_empty_symbols() {
    let src = "$primary: #333\n.container\n  color: $primary\n";
    let syms = SassExtractor
        .extract("styles/main.sass", src)
        .expect("Sass extract");
    assert!(
        syms.is_empty(),
        "Sass has no grammar; parse should return empty symbols"
    );
}

/// Ref: test_styl_parse_returns_empty_symbols
#[test]
fn test_styl_parse_returns_empty_symbols() {
    let src = "primary = #333\n.container\n  color primary\n";
    let syms = StylExtractor
        .extract("styles/main.styl", src)
        .expect("Styl extract");
    assert!(
        syms.is_empty(),
        "Styl has no grammar; parse should return empty symbols"
    );
}
