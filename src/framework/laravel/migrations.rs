use std::collections::HashMap;

use regex::Regex;

use super::php_structure;
use super::types::MigrationInfo;

/// Parse a Laravel migration file for table name and column definitions.
///
/// Uses **tree-sitter PHP** for the `Schema::create` table name when possible; column lists
/// still use the internal regex column pass. Falls back to full regex when the AST finds
/// no table name.
pub fn parse_migration(content: &str) -> Option<MigrationInfo> {
    if let Some(mut info) = php_structure::migration_from_tree(content) {
        info.columns = migration_columns_regex(content);
        if !info.table.is_empty() {
            return Some(info);
        }
    }
    parse_migration_regex(content)
}

fn parse_migration_regex(content: &str) -> Option<MigrationInfo> {
    let table_re = Regex::new(r#"Schema::create\(\s*['"]([\w]+)['"]\s*,"#).ok()?;
    let table_name = table_re.captures(content)?[1].to_string();
    Some(MigrationInfo {
        table: table_name,
        columns: migration_columns_regex(content),
    })
}

fn migration_columns_regex(content: &str) -> HashMap<String, Vec<String>> {
    let helpers = [
        "timestamps",
        "softDeletes",
        "id",
        "rememberToken",
        "nullableTimestamps",
    ];
    let Some(col_re) = Regex::new(r#"\$table->\w+\(\s*['"]([\w]+)['"]\s*\)"#).ok() else {
        return HashMap::new();
    };
    let mut columns: HashMap<String, Vec<String>> = HashMap::new();
    for cap in col_re.captures_iter(content) {
        let col_name = cap[1].to_string();
        if helpers.contains(&col_name.as_str()) {
            continue;
        }
        let full_match_end = cap.get(0).unwrap().end();
        let rest = &content[full_match_end..];
        let mut modifiers = Vec::new();
        let mod_re = Regex::new(r"^(->(\w+)\(\))+").unwrap();
        if let Some(m) = mod_re.find(rest) {
            let chain = m.as_str();
            let single_mod = Regex::new(r"->(\w+)\(\)").unwrap();
            for mc in single_mod.captures_iter(chain) {
                modifiers.push(mc[1].to_string());
            }
        }
        columns.insert(col_name, modifiers);
    }
    columns
}
