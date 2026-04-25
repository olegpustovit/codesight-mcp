use heck::ToSnakeCase;
use pluralizer::pluralize;

use super::regex::RE;

use super::php_structure;
use super::types::{ModelInfo, Relationship};

/// Guess the database table name for an Eloquent model class name.
///
/// - `User` -> `users`
/// - `Category` -> `categories`
/// - `Status` -> `statuses`
/// - `BlogPost` -> `blog_posts`
pub fn guess_table(model_name: &str) -> String {
    let snake = model_name.to_snake_case();
    match snake.rsplit_once('_') {
        Some((head, last)) => format!("{head}_{}", pluralize(last, 2, false)),
        None => pluralize(&snake, 2, false),
    }
}

/// Parse a Laravel Eloquent model file.
///
/// **tree-sitter PHP** supplies relationships and local scopes when it can; fillable and
/// remaining fields use the regex pass.
pub fn parse_model(content: &str) -> ModelInfo {
    let mut info = parse_model_regex(content);
    if let Some(ts) = php_structure::model_from_tree(content) {
        if !ts.relationships.is_empty() {
            info.relationships = ts.relationships;
        }
        if !ts.scopes.is_empty() {
            info.scopes = ts.scopes;
        }
    }
    info
}

fn parse_model_regex(content: &str) -> ModelInfo {
    let mut info = ModelInfo::default();

    for cap in RE.model_relationship.captures_iter(content) {
        let kind = cap[1].to_string();
        let target_full = cap[2].to_string();
        let target = target_full
            .rsplit('\\')
            .next()
            .unwrap_or(&target_full)
            .to_string();
        info.relationships.push(Relationship { kind, target });
    }

    if let Some(cap) = RE.model_fillable.captures(content) {
        let items_str = &cap[1];
        for item in RE.fillable_item.captures_iter(items_str) {
            info.fillable.push(item[1].to_string());
        }
    }

    for cap in RE.scope_method.captures_iter(content) {
        info.scopes.push(cap[1].to_string());
    }

    info
}
