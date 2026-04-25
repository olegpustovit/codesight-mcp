//! Integration tests: agent v1 framework core.

use rustc_hash::FxHashMap;
use std::path::Path;

use codesight_language_support::frameworks::profiles::{detect_framework, profile_to_meta};
use codesight_language_support::frameworks::{
    EnrichableSymbol, ExtraImport, FileContext, FrameworkProvider, FrameworkRegistry,
    collect_extra_imports, collect_metadata, enrich_symbol,
};

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn file_context_summary_context_full() {
    let ctx = FileContext {
        description: "Daily revenue by product".into(),
        tags: vec!["nightly".into(), "finance".into()],
        properties: vec![
            ("amount".into(), "USD value".into()),
            ("date".into(), "Revenue date".into()),
        ],
    };
    let summary = ctx.summary_context();
    assert!(summary.contains("Daily revenue by product"));
    assert!(summary.contains("nightly"));
    assert!(summary.contains("finance"));
    assert!(summary.contains("amount (USD value)"));
    assert!(summary.contains("date (Revenue date)"));
}
#[test]
fn file_context_summary_context_truncates_properties() {
    let props: Vec<(String, String)> = (0..15)
        .map(|i| (format!("prop_{i}"), format!("val_{i}")))
        .collect();
    let ctx = FileContext {
        description: String::new(),
        tags: vec![],
        properties: props,
    };
    let summary = ctx.summary_context_with_max(5);
    assert!(summary.contains("... and 10 more"));
}
#[test]
fn file_context_summary_property_without_value() {
    let ctx = FileContext {
        description: String::new(),
        tags: vec![],
        properties: vec![("bare_prop".into(), String::new())],
    };
    let summary = ctx.summary_context();
    assert!(summary.contains("bare_prop"));
    assert!(!summary.contains("("));
}
#[test]
fn file_context_file_summary_full() {
    let ctx = FileContext {
        description: "Tracks user signups".into(),
        tags: vec!["daily".into()],
        properties: vec![
            ("user_id".into(), "PK".into()),
            ("email".into(), "address".into()),
        ],
    };
    let summary = ctx.file_summary();
    assert!(summary.contains("Tracks user signups"));
    assert!(summary.contains("Tags: daily"));
    assert!(summary.contains("2 properties"));
}
#[test]
fn file_context_file_summary_truncates_description() {
    let long_desc = "A".repeat(300);
    let ctx = FileContext {
        description: long_desc,
        tags: vec![],
        properties: vec![],
    };
    let summary = ctx.file_summary();
    assert!(summary.len() < 300);
    assert!(summary.ends_with("..."));
}
#[test]
fn file_context_search_keywords() {
    let ctx = FileContext {
        description: String::new(),
        tags: vec!["nightly".into(), "finance".into()],
        properties: vec![
            ("amount".into(), String::new()),
            ("date".into(), String::new()),
        ],
    };
    let kw = ctx.search_keywords();
    assert!(kw.contains(&"nightly".to_string()));
    assert!(kw.contains(&"finance".to_string()));
    assert!(kw.contains(&"amount".to_string()));
    assert!(kw.contains(&"date".to_string()));
}
#[test]
fn file_context_empty() {
    let ctx = FileContext::default();
    assert_eq!(ctx.summary_context(), "");
    assert_eq!(ctx.file_summary(), "");
    assert!(ctx.search_keywords().is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn provider_register_adds_class() {
    let mut registry = FrameworkRegistry::new();
    let initial = registry.len();
    registry.register(|| Box::new(StubProvider::new("test", true)));
    assert_eq!(registry.len(), initial + 1);
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn provider_none_detected() {
    let mut registry = FrameworkRegistry::new();
    registry.register(|| Box::new(StubProvider::new("non_match", false)));
    let tmp = tempfile::tempdir().unwrap();
    let active = registry.discover(tmp.path());
    assert!(active.is_empty());
}
#[test]
fn provider_activates_matching() {
    let mut registry = FrameworkRegistry::new();
    registry.register(|| Box::new(StubProvider::new("match", true)));
    let tmp = tempfile::tempdir().unwrap();
    let active = registry.discover(tmp.path());
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name(), "match");
}
#[test]
fn provider_handles_load_failure() {
    let mut registry = FrameworkRegistry::new();
    registry.register(|| Box::new(FailingProvider));
    let tmp = tempfile::tempdir().unwrap();
    let active = registry.discover(tmp.path());
    // Failing load => not activated
    assert!(active.is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn enrich_sets_ecosystem_context() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(StubProvider::with_context(
        "prov1",
        "test.sql",
        "User signups",
    ))];
    let mut sym = EnrichableSymbol {
        file: "test.sql".into(),
        ..Default::default()
    };
    enrich_symbol(&mut sym, &providers);
    assert!(sym.ecosystem_context.contains("prov1"));
    assert!(sym.ecosystem_context.contains("User signups"));
}
#[test]
fn enrich_merges_keywords_no_duplicates() {
    let mut prov = StubProvider::with_context("prov1", "test.sql", "desc");
    prov.context.as_mut().unwrap().1.tags = vec!["nightly".into()];
    prov.context.as_mut().unwrap().1.properties = vec![("col_a".into(), String::new())];

    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(prov)];
    let mut sym = EnrichableSymbol {
        file: "test.sql".into(),
        keywords: vec!["nightly".into()],
        ..Default::default()
    };
    enrich_symbol(&mut sym, &providers);
    let count = sym.keywords.iter().filter(|k| *k == "nightly").count();
    assert_eq!(count, 1, "nightly should not be duplicated");
    assert!(sym.keywords.contains(&"col_a".to_string()));
}
#[test]
fn enrich_multiple_providers() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![
        Box::new(StubProvider::with_context("alpha", "f.sql", "desc_a")),
        Box::new(StubProvider::with_context("beta", "f.sql", "desc_b")),
    ];
    let mut sym = EnrichableSymbol {
        file: "f.sql".into(),
        ..Default::default()
    };
    enrich_symbol(&mut sym, &providers);
    assert!(sym.ecosystem_context.contains("alpha"));
    assert!(sym.ecosystem_context.contains("beta"));
    assert!(sym.ecosystem_context.contains(";"));
}
#[test]
fn enrich_no_match_leaves_untouched() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(StubProvider::with_context(
        "prov",
        "other.sql",
        "desc",
    ))];
    let mut sym = EnrichableSymbol {
        file: "test.sql".into(),
        ..Default::default()
    };
    enrich_symbol(&mut sym, &providers);
    assert_eq!(sym.ecosystem_context, "");
    assert!(sym.keywords.is_empty());
}
#[test]
fn enrich_empty_providers_list() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![];
    let mut sym = EnrichableSymbol::default();
    enrich_symbol(&mut sym, &providers);
    assert_eq!(sym.ecosystem_context, "");
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn collect_imports_merges_into_empty() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(StubProvider::with_imports(
        "prov",
        "routes/web.php",
        vec![ExtraImport {
            specifier: "UserController".into(),
            names: vec!["UserController".into()],
        }],
    ))];
    let mut file_imports: FxHashMap<String, Vec<ExtraImport>> = FxHashMap::default();
    collect_extra_imports(&mut file_imports, &providers);
    assert!(file_imports.contains_key("routes/web.php"));
    assert_eq!(file_imports["routes/web.php"].len(), 1);
}
#[test]
fn collect_imports_merges_existing_no_duplicates() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(StubProvider::with_imports(
        "prov",
        "routes/web.php",
        vec![ExtraImport {
            specifier: "PostController".into(),
            names: vec![],
        }],
    ))];
    let mut file_imports: FxHashMap<String, Vec<ExtraImport>> = FxHashMap::default();
    file_imports.insert(
        "routes/web.php".into(),
        vec![ExtraImport {
            specifier: "UserController".into(),
            names: vec![],
        }],
    );
    collect_extra_imports(&mut file_imports, &providers);
    assert_eq!(file_imports["routes/web.php"].len(), 2);
}
#[test]
fn collect_imports_deduplicates_same_specifier() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![Box::new(StubProvider::with_imports(
        "prov",
        "routes/web.php",
        vec![ExtraImport {
            specifier: "UserController".into(),
            names: vec![],
        }],
    ))];
    let mut file_imports: FxHashMap<String, Vec<ExtraImport>> = FxHashMap::default();
    file_imports.insert(
        "routes/web.php".into(),
        vec![ExtraImport {
            specifier: "UserController".into(),
            names: vec![],
        }],
    );
    collect_extra_imports(&mut file_imports, &providers);
    assert_eq!(file_imports["routes/web.php"].len(), 1);
}
#[test]
fn collect_imports_multiple_providers() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![
        Box::new(StubProvider::with_imports(
            "prov_a",
            "routes/web.php",
            vec![ExtraImport {
                specifier: "A".into(),
                names: vec![],
            }],
        )),
        Box::new(StubProvider::with_imports(
            "prov_b",
            "routes/web.php",
            vec![ExtraImport {
                specifier: "B".into(),
                names: vec![],
            }],
        )),
    ];
    let mut file_imports = FxHashMap::default();
    collect_extra_imports(&mut file_imports, &providers);
    assert_eq!(file_imports["routes/web.php"].len(), 2);
}
#[test]
fn collect_imports_failing_provider_does_not_crash() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![
        Box::new(FailingImportsProvider),
        Box::new(StubProvider::with_imports(
            "good",
            "routes/web.php",
            vec![ExtraImport {
                specifier: "Good".into(),
                names: vec![],
            }],
        )),
    ];
    let mut file_imports = FxHashMap::default();
    collect_extra_imports(&mut file_imports, &providers);
    assert!(file_imports.contains_key("routes/web.php"));
}
#[test]
fn collect_imports_empty_providers_list() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![];
    let mut file_imports: FxHashMap<String, Vec<ExtraImport>> = FxHashMap::default();
    file_imports.insert("existing.php".into(), vec![]);
    collect_extra_imports(&mut file_imports, &providers);
    assert!(file_imports.contains_key("existing.php"));
}
#[test]
fn collect_imports_base_provider_returns_empty() {
    let providers: Vec<Box<dyn FrameworkProvider>> =
        vec![Box::new(StubProvider::new("base", true))];
    let mut file_imports = FxHashMap::default();
    collect_extra_imports(&mut file_imports, &providers);
    assert!(file_imports.is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn framework_detect_laravel() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("artisan"), "").unwrap();
    std::fs::write(
        tmp.path().join("composer.json"),
        r#"{"require": {"laravel/framework": "^10.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "laravel");
}
#[test]
fn framework_laravel_ignore_patterns() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("artisan"), "").unwrap();
    std::fs::write(
        tmp.path().join("composer.json"),
        r#"{"require": {"laravel/framework": "^10.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert!(
        profile
            .ignore_patterns
            .iter()
            .any(|p| p.contains("vendor/"))
    );
    assert!(
        profile
            .ignore_patterns
            .iter()
            .any(|p| p.contains("node_modules/"))
    );
}
#[test]
fn framework_detect_nuxt_ts() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "nuxt");
}
#[test]
fn framework_detect_nuxt_js() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.js"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "nuxt");
}
#[test]
fn framework_nuxt_ignore_patterns() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert!(profile.ignore_patterns.iter().any(|p| p.contains(".nuxt/")));
    assert!(
        profile
            .ignore_patterns
            .iter()
            .any(|p| p.contains("node_modules/"))
    );
}
#[test]
fn framework_detect_next() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.js"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "next");
}
#[test]
fn framework_detect_next_mjs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("next.config.mjs"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "next");
}
#[test]
fn framework_detect_vue_spa() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("vite.config.ts"), "").unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/App.vue"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "vue-spa");
}
#[test]
fn framework_detect_react_spa() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("vite.config.ts"), "").unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/App.tsx"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "react-spa");
}
#[test]
fn framework_detect_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(detect_framework(tmp.path()).is_none());
}
#[test]
fn framework_overlap_returns_any_valid_profile() {
    // Contrived: both Laravel's `artisan` + `composer.json` and a
    // `nuxt.config.ts` at the same root. `detect_framework` walks the
    // language registry and returns the first match — no per-framework
    // precedence fiction. Accept either outcome so the test survives
    // registry reorderings.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("artisan"), "").unwrap();
    std::fs::write(
        tmp.path().join("composer.json"),
        r#"{"require": {"laravel/framework": "^10.0"}}"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert!(
        matches!(profile.name.as_str(), "laravel" | "nuxt"),
        "expected laravel or nuxt, got {}",
        profile.name
    );
}
#[test]
fn framework_nuxt_not_confused_with_next() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    std::fs::write(tmp.path().join("next.config.js"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    assert_eq!(profile.name, "nuxt");
}

// ── Profile metadata ─────────────────────────────────────────────────────────
#[test]
fn profile_meta_contains_name() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    let meta = profile_to_meta(&profile);
    let fp = meta["framework_profile"].as_object().unwrap();
    assert_eq!(fp["name"].as_str().unwrap(), "nuxt");
}
#[test]
fn profile_meta_contains_entry_points() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    let meta = profile_to_meta(&profile);
    let fp = meta["framework_profile"].as_object().unwrap();
    let entry = fp["entry_point_patterns"].as_array().unwrap();
    assert!(!entry.is_empty());
}
#[test]
fn profile_meta_contains_layers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("nuxt.config.ts"), "").unwrap();
    let profile = detect_framework(tmp.path()).unwrap();
    let meta = profile_to_meta(&profile);
    let fp = meta["framework_profile"].as_object().unwrap();
    let layers = fp["layer_definitions"].as_array().unwrap();
    let has_pages = layers.iter().any(|ld| {
        ld.as_object()
            .and_then(|m| m.get("name"))
            .and_then(|v| v.as_str())
            == Some("pages")
    });
    assert!(has_pages, "expected a 'pages' layer definition");
}

// ══════════════════════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════════════════════
#[test]
fn metadata_no_collision_no_warning() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![
        Box::new(MetadataProvider::new(
            "a",
            vec![("key_a".into(), serde_json::json!("val_a"))],
        )),
        Box::new(MetadataProvider::new(
            "b",
            vec![("key_b".into(), serde_json::json!("val_b"))],
        )),
    ];
    let (result, warnings) = collect_metadata(&providers);
    assert!(warnings.is_empty());
    assert!(result.contains_key("key_a"));
    assert!(result.contains_key("key_b"));
}
#[test]
fn metadata_collision_logs_warning() {
    let providers: Vec<Box<dyn FrameworkProvider>> = vec![
        Box::new(MetadataProvider::new(
            "first",
            vec![("shared_key".into(), serde_json::json!("first_val"))],
        )),
        Box::new(MetadataProvider::new(
            "second",
            vec![("shared_key".into(), serde_json::json!("second_val"))],
        )),
    ];
    let (result, warnings) = collect_metadata(&providers);
    // Last writer wins
    assert_eq!(result["shared_key"], serde_json::json!("second_val"));
    assert!(!warnings.is_empty());
    let w = &warnings[0];
    assert!(
        w.contains("overwrites"),
        "warning should mention overwrites: {w}"
    );
    assert!(
        w.contains("second"),
        "warning should mention provider name: {w}"
    );
}
#[test]
fn metadata_empty_not_merged() {
    let providers: Vec<Box<dyn FrameworkProvider>> =
        vec![Box::new(MetadataProvider::new("empty", vec![]))];
    let (result, _) = collect_metadata(&providers);
    assert!(result.is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════
// Test helpers
// ══════════════════════════════════════════════════════════════════════════════

struct StubProvider {
    provider_name: String,
    should_detect: bool,
    context: Option<(String, FileContext)>, // (file, context)
    imports: Option<(String, Vec<ExtraImport>)>, // (file, imports)
}

impl StubProvider {
    fn new(name: &str, detect: bool) -> Self {
        Self {
            provider_name: name.into(),
            should_detect: detect,
            context: None,
            imports: None,
        }
    }

    fn with_context(name: &str, file: &str, desc: &str) -> Self {
        Self {
            provider_name: name.into(),
            should_detect: true,
            context: Some((
                file.into(),
                FileContext {
                    description: desc.into(),
                    tags: vec![],
                    properties: vec![],
                },
            )),
            imports: None,
        }
    }

    fn with_imports(name: &str, file: &str, imports: Vec<ExtraImport>) -> Self {
        Self {
            provider_name: name.into(),
            should_detect: true,
            context: None,
            imports: Some((file.into(), imports)),
        }
    }
}

impl FrameworkProvider for StubProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }
    fn detect(&self, _root: &Path) -> bool {
        self.should_detect
    }
    fn load(&mut self, _root: &Path) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_file_context(&self, rel_path: &str) -> Option<FileContext> {
        self.context
            .as_ref()
            .filter(|(f, _)| f == rel_path)
            .map(|(_, ctx)| ctx.clone())
    }
    fn get_extra_imports(&self) -> anyhow::Result<FxHashMap<String, Vec<ExtraImport>>> {
        Ok(match &self.imports {
            Some((file, imps)) => {
                let mut m = FxHashMap::default();
                m.insert(file.clone(), imps.clone());
                m
            }
            None => FxHashMap::default(),
        })
    }
}

struct FailingProvider;

impl FrameworkProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }
    fn detect(&self, _root: &Path) -> bool {
        true
    }
    fn load(&mut self, _root: &Path) -> anyhow::Result<()> {
        anyhow::bail!("load failure")
    }
    fn get_file_context(&self, _rel_path: &str) -> Option<FileContext> {
        None
    }
}

struct FailingImportsProvider;

impl FrameworkProvider for FailingImportsProvider {
    fn name(&self) -> &str {
        "failing_imports"
    }
    fn detect(&self, _root: &Path) -> bool {
        true
    }
    fn load(&mut self, _root: &Path) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_file_context(&self, _rel_path: &str) -> Option<FileContext> {
        None
    }
    fn get_extra_imports(&self) -> anyhow::Result<FxHashMap<String, Vec<ExtraImport>>> {
        anyhow::bail!("intentional error in get_extra_imports")
    }
}

struct MetadataProvider {
    provider_name: String,
    meta: Vec<(String, serde_json::Value)>,
}

impl MetadataProvider {
    fn new(name: &str, meta: Vec<(String, serde_json::Value)>) -> Self {
        Self {
            provider_name: name.into(),
            meta,
        }
    }
}

impl FrameworkProvider for MetadataProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }
    fn detect(&self, _root: &Path) -> bool {
        true
    }
    fn load(&mut self, _root: &Path) -> anyhow::Result<()> {
        Ok(())
    }
    fn get_file_context(&self, _rel_path: &str) -> Option<FileContext> {
        None
    }
    fn get_metadata(&self) -> FxHashMap<String, serde_json::Value> {
        self.meta.iter().cloned().collect()
    }
}
