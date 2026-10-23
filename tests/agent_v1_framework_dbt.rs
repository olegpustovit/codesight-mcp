//! Integration tests: agent v1 framework dbt.

use rustc_hash::FxHashMap;

use dowser_language_support::frameworks::FrameworkProvider;
use dowser_language_support::languages::sql::dbt::*;

// ── _detect_dbt_project ─────────────────────────────────────────────────────
#[test]
fn dbt_detect_root_level() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("dbt_project.yml"), "").unwrap();
    assert!(detect_dbt_project(tmp.path()).is_some());
}
#[test]
fn dbt_detect_one_level_deep() {
    let tmp = tempfile::tempdir().unwrap();
    let sub = tmp.path().join("DBT");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("dbt_project.yml"), "").unwrap();
    assert!(detect_dbt_project(tmp.path()).is_some());
}
#[test]
fn dbt_detect_too_deep() {
    let tmp = tempfile::tempdir().unwrap();
    let deep = tmp.path().join("a/b");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("dbt_project.yml"), "").unwrap();
    assert!(detect_dbt_project(tmp.path()).is_none());
}
#[test]
fn dbt_detect_no_project() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(detect_dbt_project(tmp.path()).is_none());
}
#[test]
fn dbt_detect_ignores_hidden_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let hidden = tmp.path().join(".hidden");
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::write(hidden.join("dbt_project.yml"), "").unwrap();
    assert!(detect_dbt_project(tmp.path()).is_none());
}

// ── _parse_doc_blocks ───────────────────────────────────────────────────────
#[test]
fn dbt_parse_single_doc_block() {
    let blocks =
        parse_doc_blocks_from_content("{% docs my_model %}Tracks daily revenue.{% enddocs %}");
    assert_eq!(blocks.get("my_model").unwrap(), "Tracks daily revenue.");
}
#[test]
fn dbt_parse_multiple_doc_blocks() {
    let content = "{% docs model_a %}Desc A.{% enddocs %}\n{% docs model_b %}Desc B.{% enddocs %}";
    let blocks = parse_doc_blocks_from_content(content);
    assert_eq!(blocks.len(), 2);
    assert!(blocks.contains_key("model_a"));
    assert!(blocks.contains_key("model_b"));
}
#[test]
fn dbt_parse_multiline_doc_block() {
    let content = "{% docs my_doc %}\nLine 1.\nLine 2.\n{% enddocs %}";
    let blocks = parse_doc_blocks_from_content(content);
    let body = blocks.get("my_doc").unwrap();
    assert!(body.contains("Line 1."));
    assert!(body.contains("Line 2."));
}
#[test]
fn dbt_parse_no_docs_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let blocks = parse_doc_blocks(&tmp.path().join("nonexistent"));
    assert!(blocks.is_empty());
}
#[test]
fn dbt_parse_multiple_docs_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("docs_a");
    let dir_b = tmp.path().join("docs_b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(
        dir_a.join("a.md"),
        "{% docs doc_a %}A content.{% enddocs %}",
    )
    .unwrap();
    std::fs::write(
        dir_b.join("b.md"),
        "{% docs doc_b %}B content.{% enddocs %}",
    )
    .unwrap();
    let blocks = parse_doc_blocks_multi(&[&dir_a, &dir_b]);
    assert_eq!(blocks.len(), 2);
}

// ── _resolve_description ────────────────────────────────────────────────────
#[test]
fn dbt_resolve_doc_reference_single_quotes() {
    let mut blocks = FxHashMap::default();
    blocks.insert(
        "my_doc".to_string(),
        "This is the resolved text.".to_string(),
    );
    let result = resolve_description("{{ doc('my_doc') }}", &blocks);
    assert_eq!(result, "This is the resolved text.");
}
#[test]
fn dbt_resolve_doc_reference_double_quotes() {
    let mut blocks = FxHashMap::default();
    blocks.insert("my_doc".to_string(), "Resolved.".to_string());
    let result = resolve_description("{{ doc(\"my_doc\") }}", &blocks);
    assert_eq!(result, "Resolved.");
}
#[test]
fn dbt_resolve_plain_description() {
    let blocks = FxHashMap::default();
    let result = resolve_description("A plain description.", &blocks);
    assert_eq!(result, "A plain description.");
}
#[test]
fn dbt_resolve_missing_reference() {
    let blocks = FxHashMap::default();
    let result = resolve_description("{{ doc('missing') }}", &blocks);
    assert_eq!(result, "");
}
#[test]
fn dbt_resolve_empty() {
    let blocks = FxHashMap::default();
    let result = resolve_description("", &blocks);
    assert_eq!(result, "");
}

// ── _resolve_description inline substitution ────────────────────────────────
#[test]
fn dbt_resolve_preserves_surrounding_text() {
    let mut blocks = FxHashMap::default();
    blocks.insert("my_doc".into(), "resolved text".into());
    let result = resolve_description("Prefix {{ doc('my_doc') }} suffix", &blocks);
    assert_eq!(result, "Prefix resolved text suffix");
}
#[test]
fn dbt_resolve_multiple_doc_refs() {
    let mut blocks = FxHashMap::default();
    blocks.insert("doc_a".into(), "AAA".into());
    blocks.insert("doc_b".into(), "BBB".into());
    let result = resolve_description("{{ doc('doc_a') }} and {{ doc('doc_b') }}", &blocks);
    assert_eq!(result, "AAA and BBB");
}
#[test]
fn dbt_resolve_missing_ref_becomes_empty_inline() {
    let blocks = FxHashMap::default();
    let result = resolve_description("Before {{ doc('missing') }} after", &blocks);
    assert_eq!(result, "Before  after");
}
#[test]
fn dbt_resolve_plain_text_unchanged_inline() {
    let mut blocks = FxHashMap::default();
    blocks.insert("something".into(), "val".into());
    let result = resolve_description("No doc refs here.", &blocks);
    assert_eq!(result, "No doc refs here.");
}
#[test]
fn dbt_resolve_full_doc_ref_wrapped() {
    let mut blocks = FxHashMap::default();
    blocks.insert("my_doc".into(), "resolved".into());
    let result = resolve_description("{{ doc('my_doc') }}", &blocks);
    assert_eq!(result, "resolved");
}

// ── YAML schema parsing ────────────────────────────────────────────────────
#[test]
fn dbt_parse_yml_model_metadata() {
    let yaml = r#"
models:
  - name: fct_orders
    description: "Order fact table"
    config:
      tags: ['nightly', 'core']
    columns:
      - name: order_id
        description: "Primary key"
      - name: amount
        description: "Order total in USD"
"#;
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("fct_orders").unwrap();
    assert_eq!(m.description, "Order fact table");
    assert!(m.tags.contains(&"nightly".to_string()));
    assert!(m.tags.contains(&"core".to_string()));
    assert_eq!(m.columns.get("order_id").unwrap(), "Primary key");
    assert_eq!(m.columns.get("amount").unwrap(), "Order total in USD");
}
#[test]
fn dbt_parse_yml_with_doc_refs() {
    let yaml = r#"
models:
  - name: dim_customer
    description: "{{ doc('dim_customer_doc') }}"
    columns:
      - name: customer_id
        description: "{{ doc('customer_id_doc') }}"
"#;
    let mut blocks = FxHashMap::default();
    blocks.insert("dim_customer_doc".into(), "Customer dimension".into());
    blocks.insert("customer_id_doc".into(), "Unique customer ID".into());
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("dim_customer").unwrap();
    assert_eq!(m.description, "Customer dimension");
    assert_eq!(m.columns.get("customer_id").unwrap(), "Unique customer ID");
}
#[test]
fn dbt_parse_yml_malformed() {
    let yaml = "not: [valid yaml: {{";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    assert!(models.is_empty());
}
#[test]
fn dbt_parse_yml_no_models_key() {
    let yaml = "sources:\n  - name: raw\n";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    assert!(models.is_empty());
}
#[test]
fn dbt_parse_yml_missing_name() {
    let yaml = "models:\n  - description: No name\n";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    assert!(models.is_empty());
}
#[test]
fn dbt_parse_yml_multiple_models() {
    let yaml = r#"
models:
  - name: model_a
    description: A
  - name: model_b
    description: B
"#;
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    assert_eq!(models.len(), 2);
}

// ── dbt tags merge ─────────────────────────────────────────────────────────
#[test]
fn dbt_tags_top_level_only() {
    let yaml = "models:\n  - name: m\n    tags: ['daily', 'finance']\n";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("m").unwrap();
    assert_eq!(m.tags, vec!["daily", "finance"]);
}
#[test]
fn dbt_tags_config_only() {
    let yaml = "models:\n  - name: m\n    config:\n      tags: ['nightly']\n";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("m").unwrap();
    assert_eq!(m.tags, vec!["nightly"]);
}
#[test]
fn dbt_tags_both_merged_deduplicated() {
    let yaml = r#"
models:
  - name: m
    tags: ['daily', 'core']
    config:
      tags: ['core', 'nightly']
"#;
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("m").unwrap();
    assert_eq!(m.tags, vec!["daily", "core", "nightly"]);
}
#[test]
fn dbt_tags_invalid_top_level_ignored() {
    let yaml = "models:\n  - name: m\n    tags: not_a_list\n    config:\n      tags: ['valid']\n";
    let blocks = FxHashMap::default();
    let models = parse_yml_content(yaml, &blocks);
    let m = models.get("m").unwrap();
    assert_eq!(m.tags, vec!["valid"]);
}

// ── DbtModelMetadata ────────────────────────────────────────────────────────
#[test]
fn dbt_model_metadata_to_file_context() {
    let mut columns = FxHashMap::default();
    columns.insert("order_id".into(), "PK".into());
    columns.insert("amount".into(), "USD".into());
    let meta = DbtModelMetadata {
        description: "Order fact table".into(),
        tags: vec!["nightly".into()],
        columns,
    };
    let ctx = meta.to_file_context();
    assert_eq!(ctx.description, "Order fact table");
    assert_eq!(ctx.tags, vec!["nightly"]);
    assert!(
        ctx.properties
            .iter()
            .any(|(k, v)| k == "order_id" && v == "PK")
    );
    assert!(
        ctx.properties
            .iter()
            .any(|(k, v)| k == "amount" && v == "USD")
    );
}
#[test]
fn dbt_model_metadata_to_file_context_empty() {
    let meta = DbtModelMetadata::default();
    let ctx = meta.to_file_context();
    assert!(ctx.description.is_empty());
    assert!(ctx.tags.is_empty());
    assert!(ctx.properties.is_empty());
}

// ── DbtContextProvider integration ──────────────────────────────────────────
#[test]
fn dbt_full_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Create dbt_project.yml
    std::fs::write(root.join("dbt_project.yml"), "name: test_project\n").unwrap();

    // Create docs directory with doc block
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(
        root.join("docs/docs.md"),
        "{% docs fct_orders %}All customer orders.{% enddocs %}",
    )
    .unwrap();

    // Create models directory with schema YAML
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        r#"models:
  - name: fct_orders
    description: "{{ doc('fct_orders') }}"
    config:
      tags: ['nightly']
    columns:
      - name: order_id
        description: "Primary key"
"#,
    )
    .unwrap();

    // Create model SQL file
    std::fs::write(root.join("models/fct_orders.sql"), "SELECT 1").unwrap();

    let mut provider = DbtContextProvider::new();
    assert!(provider.detect(root));
    provider.load(root).unwrap();

    let ctx = provider.get_file_context("models/fct_orders.sql").unwrap();
    assert_eq!(ctx.description, "All customer orders.");
    assert!(ctx.tags.contains(&"nightly".to_string()));
    assert!(ctx.properties.iter().any(|(k, _)| k == "order_id"));
}
#[test]
fn dbt_get_file_context_by_stem() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: test\n").unwrap();
    std::fs::create_dir_all(root.join("models/staging")).unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        "models:\n  - name: my_model\n    description: Test\n",
    )
    .unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();

    assert!(provider.get_file_context("models/my_model.sql").is_some());
    assert!(
        provider
            .get_file_context("models/staging/my_model.sql")
            .is_some()
    );
}
#[test]
fn dbt_get_file_context_outside_model_path() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: test\n").unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        "models:\n  - name: my_model\n    description: Test\n",
    )
    .unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();

    // Outside model dirs
    assert!(provider.get_file_context("my_model.sql").is_none());
    assert!(provider.get_file_context("scripts/my_model.sql").is_none());
    // Inside models/
    assert!(provider.get_file_context("models/my_model.sql").is_some());
}
#[test]
fn dbt_get_file_context_no_match() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: test\n").unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();

    assert!(provider.get_file_context("models/unknown.sql").is_none());
}

#[test]
fn dbt_snapshot_exposes_model_context_without_provider() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: sample\n").unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        "models:\n  - name: my_model\n    description: Example model\n",
    )
    .unwrap();

    let snapshot = DbtSnapshot::load(root).expect("dbt snapshot");
    let ctx = render_file_context(&snapshot, "models/my_model.sql").expect("model context");
    assert!(ctx.description.contains("Example model"));
    assert_eq!(snapshot.stats().models_with_metadata, 1);
}

#[test]
fn dbt_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: test\n").unwrap();
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(
        root.join("docs/docs.md"),
        "{% docs doc_a %}A{% enddocs %}\n{% docs doc_b %}B{% enddocs %}",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        "models:\n  - name: m1\n    description: Model1\n",
    )
    .unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();
    let stats = provider.stats();
    assert_eq!(stats.doc_blocks, 2);
    assert_eq!(stats.models_with_metadata, 1);
}
#[test]
fn dbt_detect_returns_false_for_non_dbt() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = DbtContextProvider::new();
    assert!(!provider.detect(tmp.path()));
}
#[test]
fn dbt_custom_model_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(
        root.join("dbt_project.yml"),
        "name: test\nmodel-paths: ['src/models']\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src/models")).unwrap();
    std::fs::write(
        root.join("src/models/schema.yml"),
        "models:\n  - name: custom_model\n    description: Custom\n",
    )
    .unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();

    assert!(
        provider
            .get_file_context("src/models/custom_model.sql")
            .is_some()
    );
}
#[test]
fn dbt_docs_inside_models_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::write(root.join("dbt_project.yml"), "name: test\n").unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(
        root.join("models/inline_doc.md"),
        "{% docs inline_model %}Inline description.{% enddocs %}",
    )
    .unwrap();
    std::fs::write(
        root.join("models/schema.yml"),
        "models:\n  - name: inline_model\n    description: \"{{ doc('inline_model') }}\"\n",
    )
    .unwrap();

    let mut provider = DbtContextProvider::new();
    provider.load(root).unwrap();

    let ctx = provider
        .get_file_context("models/inline_model.sql")
        .unwrap();
    assert_eq!(ctx.description, "Inline description.");
}
