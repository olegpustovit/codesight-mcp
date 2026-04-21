//! Integration tests: sql language.

use std::path::Path;

use codesight_language_support::LanguageRegistry;
use codesight_language_support::languages::sql::extractor::SqlExtractor;
use codesight_core::SymbolKind;

// ── Inline fixtures (matching reference Python) ─────────────────────────────

const SQL_DDL: &str = "\
-- Orders table stores all customer orders
CREATE TABLE orders (
    id          INT PRIMARY KEY,
    customer_id INT NOT NULL,
    status      VARCHAR(20),
    total       DECIMAL(10,2)
);

-- Active orders view
CREATE VIEW active_orders AS
    SELECT * FROM orders WHERE status = 'active';

-- Get order total function
CREATE FUNCTION get_order_total(p_id INT)
RETURNS DECIMAL
BEGIN
    DECLARE v_total DECIMAL;
    SELECT total INTO v_total FROM orders WHERE id = p_id;
    RETURN v_total;
END;

CREATE INDEX idx_orders_status ON orders(status);

CREATE SCHEMA analytics;
";

const SQL_WITH_CTE: &str = "\
WITH ranked AS (
    SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM orders
)
SELECT * FROM ranked;
";

const SQL_DBT_JINJA: &str = "\
-- dbt model: staging orders
SELECT
    id,
    {{ dbt_utils.star(from=ref('raw_orders')) }},
    status
FROM {{ ref('raw_orders') }}
WHERE {% if is_incremental() %}
    updated_at > (SELECT MAX(updated_at) FROM {{ this }})
{% endif %}
    status != 'deleted'
";

const DBT_MACRO: &str = "\
{#
/**
 * Deduplicate rows in a final model.
 * Keeps the first row per partition.
 */
#}
{% macro macro_dedupe_final_model(source_cte, partition_by, order_by) %}
    select
        * exclude rnk
    from (
        select
            *,
            row_number() over (
                partition by {{partition_by}} order by {{order_by}}
            ) as rnk
        from {{source_cte}}
        qualify rnk = 1
    )
{% endmacro %}
";

const DBT_TEST: &str = "\
{% test datashare_safe_row_count_test(model, source_name, source_table_name) %}

{%- set source_relation = source(source_name, source_table_name) -%}

select count(*) from {{ model }}

{% endtest %}
";

const DBT_SNAPSHOT: &str = "\
{% snapshot snap_zoominfo_contacts %}
{{
    config(unique_key = \"CONTACT_ZOOMINFO_ID\",
        strategy = \"check\",
        check_cols = \"all\")
}}
select * from {{ source('zoominfo', 'contacts') }}
{% endsnapshot %}
";

const DBT_MATERIALIZATION: &str = "\
{% materialization cortex_agent, adapter='snowflake' %}
    {%- set target_relation = this -%}
    {{ return({'relations': [target_relation]}) }}
{% endmaterialization %}
";

const DBT_MULTI_MACRO: &str = "\
-- Helper macro
{% macro helper_one(x) %}
    select {{x}}
{% endmacro %}

{# Second helper #}
{% macro helper_two(a, b) %}
    select {{a}}, {{b}}
{% endmacro %}
";

// ── Helper ──────────────────────────────────────────────────────────────────

fn extract(source: &str, file: &str) -> Vec<codesight_core::StoredSymbol> {
    SqlExtractor
        .extract(file, source)
        .expect("extraction should not fail")
}

fn find_by_name<'a>(
    syms: &'a [codesight_core::StoredSymbol],
    name: &str,
) -> Option<&'a codesight_core::StoredSymbol> {
    syms.iter().find(|s| s.name == name)
}

// ── DDL / tree-sitter tests ─────────────────────────────────────────────────
#[test]
fn test_sql_parser_available() {
    // Smoke test: SqlExtractor must be constructible and grammar must load.
    let syms = SqlExtractor.extract("smoke.sql", "SELECT 1;");
    assert!(syms.is_ok(), "SQL grammar failed to load");
}
#[test]
fn test_sql_symbols_extracted() {
    let syms = extract(SQL_DDL, "test.sql");
    assert!(
        syms.len() >= 4,
        "expected >= 4 symbols, got {}: {:?}",
        syms.len(),
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
#[test]
fn test_sql_table_as_type() {
    let syms = extract(SQL_DDL, "test.sql");
    let table = find_by_name(&syms, "orders");
    assert!(
        table.is_some(),
        "orders table not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(table.unwrap().kind, SymbolKind::Type);
}
#[test]
fn test_sql_view_extracted() {
    let syms = extract(SQL_DDL, "test.sql");
    let view = find_by_name(&syms, "active_orders");
    assert!(
        view.is_some(),
        "active_orders view not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(view.unwrap().kind, SymbolKind::Type);
}
#[test]
fn test_sql_function_extracted() {
    let syms = extract(SQL_DDL, "test.sql");
    let func = find_by_name(&syms, "get_order_total");
    assert!(
        func.is_some(),
        "get_order_total not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(func.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_sql_index_extracted() {
    let syms = extract(SQL_DDL, "test.sql");
    let idx = find_by_name(&syms, "idx_orders_status");
    assert!(
        idx.is_some(),
        "idx_orders_status not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(idx.unwrap().kind, SymbolKind::Type);
}
#[test]
fn test_sql_schema_extracted() {
    let syms = extract(SQL_DDL, "test.sql");
    let schema = find_by_name(&syms, "analytics");
    assert!(
        schema.is_some(),
        "analytics schema not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(schema.unwrap().kind, SymbolKind::Type);
}
#[test]
fn test_sql_cte_extracted() {
    let syms = extract(SQL_WITH_CTE, "cte.sql");
    let cte = find_by_name(&syms, "ranked");
    assert!(
        cte.is_some(),
        "ranked CTE not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(cte.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_sql_function_signature() {
    let syms = extract(SQL_DDL, "test.sql");
    let func = find_by_name(&syms, "get_order_total");
    assert!(func.is_some(), "get_order_total not found");
    let sig = &func.unwrap().signature;
    assert!(
        sig.contains("CREATE FUNCTION"),
        "signature missing CREATE FUNCTION: {sig}"
    );
    assert!(sig.contains("RETURNS"), "signature missing RETURNS: {sig}");
}
#[test]
fn test_sql_table_signature() {
    let syms = extract(SQL_DDL, "test.sql");
    let table = find_by_name(&syms, "orders");
    assert!(table.is_some(), "orders not found");
    let sig = &table.unwrap().signature;
    assert!(
        sig.contains("CREATE TABLE orders"),
        "signature missing CREATE TABLE orders: {sig}"
    );
}
#[test]
fn test_sql_table_docstring() {
    let syms = extract(SQL_DDL, "test.sql");
    let table = find_by_name(&syms, "orders");
    assert!(table.is_some(), "orders not found");
    let summary = &table.unwrap().summary;
    assert!(
        summary.contains("Orders table"),
        "summary missing 'Orders table': {summary}"
    );
}

// ── Jinja / dbt tolerance tests ─────────────────────────────────────────────
#[test]
fn test_sql_partial_parse_tolerant() {
    let result = SqlExtractor.extract("staging_orders.sql", SQL_DBT_JINJA);
    assert!(
        result.is_ok(),
        "extraction should not error on Jinja SQL: {:?}",
        result.err()
    );
    // May return 0 or more symbols; the key point is no panic/error.
}
/// the extractor. (The reference tests Python-internal strip_jinja / is_jinja_sql;
/// we port the observable behavior: the extractor handles Jinja SQL gracefully.)
#[test]
fn test_sql_jinja_preprocessor() {
    let jinja_sql = "SELECT * FROM {{ ref('orders') }} WHERE {% if true %}1=1{% endif %}";
    let result = SqlExtractor.extract("jinja_model.sql", jinja_sql);
    assert!(
        result.is_ok(),
        "extraction should not error on Jinja expressions"
    );
}
/// should parse without errors.
#[test]
fn test_sql_jinja_dbt_model_parses() {
    let dbt_model = "\
-- dbt model with ref and config
{{ config(materialized='table') }}

CREATE TABLE {{ ref('stg_orders') }} AS
SELECT
    id,
    {{ dbt_utils.star(from=ref('raw_orders')) }}
FROM {{ ref('raw_orders') }}
";
    let result = SqlExtractor.extract("models/orders.sql", dbt_model);
    assert!(
        result.is_ok(),
        "dbt model with Jinja should not error: {:?}",
        result.err()
    );
}

// ── Extension / registry tests ──────────────────────────────────────────────
#[test]
fn test_sql_extension_registered() {
    let registry = LanguageRegistry::default();
    let ext = registry.extractor_for_path(Path::new("test.sql"));
    assert!(
        ext.is_some(),
        ".sql should be recognized by LanguageRegistry"
    );
    assert_eq!(ext.unwrap().language_name(), "sql");
}
#[test]
fn test_sql_language_in_registry() {
    let registry = LanguageRegistry::default();
    let ext = registry.extractor_for_path(Path::new("anything.sql"));
    assert!(ext.is_some(), "sql language should be in registry");
}

// ── dbt directive extraction tests ──────────────────────────────────────────
#[test]
fn test_dbt_macro_extracted() {
    let syms = extract(DBT_MACRO, "macros/dedupe.sql");
    let m = find_by_name(&syms, "macro_dedupe_final_model");
    assert!(
        m.is_some(),
        "macro_dedupe_final_model not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(m.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_dbt_macro_signature() {
    let syms = extract(DBT_MACRO, "macros/dedupe.sql");
    let m = find_by_name(&syms, "macro_dedupe_final_model");
    assert!(m.is_some(), "macro_dedupe_final_model not found");
    let sig = &m.unwrap().signature;
    assert!(
        sig.contains("macro_dedupe_final_model"),
        "signature missing macro name: {sig}"
    );
    assert!(
        sig.contains("source_cte"),
        "signature missing source_cte param: {sig}"
    );
}
#[test]
fn test_dbt_macro_docstring() {
    let syms = extract(DBT_MACRO, "macros/dedupe.sql");
    let m = find_by_name(&syms, "macro_dedupe_final_model");
    assert!(m.is_some(), "macro_dedupe_final_model not found");
    let summary = &m.unwrap().summary;
    assert!(
        summary.contains("Deduplicate"),
        "summary missing 'Deduplicate': {summary}"
    );
}
#[test]
fn test_dbt_macro_line_range() {
    let syms = extract(DBT_MACRO, "macros/dedupe.sql");
    let m = find_by_name(&syms, "macro_dedupe_final_model");
    assert!(m.is_some(), "macro_dedupe_final_model not found");
    let m = m.unwrap();
    // The {% macro ... %} line is line 7 in the fixture (1-indexed).
    // Our regex counts lines-before start_byte, so the exact number may differ
    // by one depending on counting convention. Assert it is in the right area
    // and spans multiple lines.
    assert!(
        m.start_line >= 6 && m.start_line <= 8,
        "expected start_line near 7, got {}",
        m.start_line
    );
    assert!(
        m.end_line > m.start_line,
        "macro should span multiple lines: start={}, end={}",
        m.start_line,
        m.end_line
    );
}
#[test]
fn test_dbt_test_extracted() {
    let syms = extract(DBT_TEST, "tests/row_count.sql");
    let t = find_by_name(&syms, "datashare_safe_row_count_test");
    assert!(
        t.is_some(),
        "datashare_safe_row_count_test not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(t.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_dbt_snapshot_extracted() {
    let syms = extract(DBT_SNAPSHOT, "snapshots/contacts.sql");
    let snap = find_by_name(&syms, "snap_zoominfo_contacts");
    assert!(
        snap.is_some(),
        "snap_zoominfo_contacts not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(snap.unwrap().kind, SymbolKind::Type);
}
#[test]
fn test_dbt_materialization_extracted() {
    let syms = extract(DBT_MATERIALIZATION, "macros/cortex_agent.sql");
    let mat = find_by_name(&syms, "cortex_agent");
    assert!(
        mat.is_some(),
        "cortex_agent not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(mat.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_dbt_multiple_macros() {
    let syms = extract(DBT_MULTI_MACRO, "macros/helpers.sql");
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"helper_one"),
        "helper_one not found in {names:?}"
    );
    assert!(
        names.contains(&"helper_two"),
        "helper_two not found in {names:?}"
    );
}
#[test]
fn test_dbt_macro_with_whitespace_variants() {
    let src = "{%- macro trimmed_macro(x, y) -%}\n  select {{x}}\n{%- endmacro -%}\n";
    let syms = extract(src, "macros/trim.sql");
    let m = find_by_name(&syms, "trimmed_macro");
    assert!(
        m.is_some(),
        "trimmed_macro not found in symbols: {:?}",
        syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert_eq!(m.unwrap().kind, SymbolKind::Function);
}
#[test]
fn test_dbt_snapshot_no_params() {
    let syms = extract(DBT_SNAPSHOT, "snapshots/contacts.sql");
    let snap = find_by_name(&syms, "snap_zoominfo_contacts");
    assert!(snap.is_some(), "snap_zoominfo_contacts not found");
    let sig = &snap.unwrap().signature;
    assert!(
        sig.contains("snapshot"),
        "signature missing 'snapshot': {sig}"
    );
}
#[test]
fn test_plain_sql_unaffected() {
    let syms = extract(SQL_DDL, "test.sql");
    assert!(
        syms.len() >= 4,
        "expected >= 4 symbols for plain DDL, got {}",
        syms.len()
    );
    let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"orders"), "orders not found in {names:?}");
    assert!(
        names.contains(&"active_orders"),
        "active_orders not found in {names:?}"
    );
}
