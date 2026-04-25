//! Tree-sitter (**PHP**) structural passes for Laravel helpers.
//!
//! Regex remains as fallback when the AST cannot produce the same facts (partial parse,
//! unusual DSL shapes). Column modifier chains in migrations still use regex augmentation
//! when the tree does not yield columns.

use rustc_hash::FxHashMap;

use tree_sitter::Node;

use super::regex::RE as LRE;
use super::types::{MigrationInfo, ModelInfo, Relationship, RouteInfo};

fn parse_php(content: &str) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .ok()?;
    parser.parse(content, None)
}

fn node_text<'a>(node: Node<'a>, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

/// Structural migration parse: `Schema::create('table', ...)` first string argument.
pub(super) fn migration_from_tree(content: &str) -> Option<MigrationInfo> {
    let tree = parse_php(content)?;
    let mut table: Option<String> = None;
    find_schema_create_table(tree.root_node(), content, &mut table);
    let table = table?;
    if table.is_empty() {
        return None;
    }
    Some(MigrationInfo {
        table,
        columns: FxHashMap::default(),
    })
}

fn find_schema_create_table(node: Node<'_>, source: &str, table: &mut Option<String>) {
    if table.is_some() {
        return;
    }
    if let Some(t) = try_schema_create_table_name(node, source) {
        *table = Some(t);
        return;
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        find_schema_create_table(child, source, table);
    }
}

fn try_schema_create_table_name(node: Node<'_>, source: &str) -> Option<String> {
    let t = node_text(node, source)?.trim();
    if !t.contains("Schema::create(") && !t.contains("\\Schema::create(") {
        return None;
    }
    first_string_argument(node, source).or_else(|| schema_create_first_table_regex(t))
}

fn schema_create_first_table_regex(call_text: &str) -> Option<String> {
    LRE.schema_create
        .captures(call_text)
        .and_then(|c| c.name("t").map(|m| m.as_str().to_string()))
}

/// First single/double-quoted string among call argument children.
fn first_string_argument(call_node: Node<'_>, source: &str) -> Option<String> {
    let mut c = call_node.walk();
    for child in call_node.named_children(&mut c) {
        if child.kind() == "arguments" || child.kind() == "argument_list" {
            let mut c2 = child.walk();
            for arg in child.named_children(&mut c2) {
                if let Some(s) = string_literal_value(arg, source) {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn string_literal_value(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "string" => {
            let raw = node_text(node, source)?;
            let raw = raw.trim();
            if raw.len() >= 2 {
                let q = raw.chars().next()?;
                if (q == '\'' || q == '"') && raw.ends_with(q) {
                    return Some(raw[1..raw.len() - 1].to_string());
                }
            }
            None
        }
        "encapsed_string" => {
            let raw = node_text(node, source)?;
            let raw = raw.trim();
            if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
                return Some(raw[1..raw.len() - 1].to_string());
            }
            if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
                return Some(raw[1..raw.len() - 1].to_string());
            }
            None
        }
        _ => None,
    }
}

// ── Model (relationships / scopes from tree; fillable augmented by caller) ──

pub(super) fn model_from_tree(content: &str) -> Option<ModelInfo> {
    let tree = parse_php(content)?;
    let mut info = ModelInfo::default();
    walk_model_class(tree.root_node(), content, &mut info);
    if info.relationships.is_empty() && info.scopes.is_empty() {
        return None;
    }
    Some(info)
}

fn walk_model_class(node: Node<'_>, source: &str, info: &mut ModelInfo) {
    if node.kind() == "method_declaration" {
        collect_relationships_from_method(node, source, info);
        collect_scope_from_method(node, source, info);
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        walk_model_class(child, source, info);
    }
}

fn method_name(node: Node<'_>, source: &str) -> Option<String> {
    let name = node.child_by_field_name("name")?;
    let t = node_text(name, source)?;
    Some(t.to_string())
}

fn collect_scope_from_method(node: Node<'_>, source: &str, info: &mut ModelInfo) {
    let Some(n) = method_name(node, source) else {
        return;
    };
    let Some(rest) = n.strip_prefix("scope") else {
        return;
    };
    if rest.is_empty()
        || !rest
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    {
        return;
    }
    info.scopes.push(rest.to_string());
}

fn collect_relationships_from_method(node: Node<'_>, source: &str, info: &mut ModelInfo) {
    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut c = body.walk();
    for st in body.named_children(&mut c) {
        if st.kind() != "return_statement" {
            continue;
        }
        let mut c2 = st.walk();
        for ch in st.named_children(&mut c2) {
            find_relationship_in_expr(ch, source, info);
        }
    }
}

fn find_relationship_in_expr(node: Node<'_>, source: &str, info: &mut ModelInfo) {
    let Some(txt) = node_text(node, source) else {
        return;
    };
    let names = [
        "hasMany",
        "hasOne",
        "belongsTo",
        "belongsToMany",
        "hasManyThrough",
        "hasOneThrough",
        "morphMany",
        "morphOne",
        "morphTo",
        "morphToMany",
        "morphedByMany",
    ];
    for kind in names {
        let needle = format!("->{kind}(");
        if !txt.contains(&needle) {
            continue;
        }
        if let Some(target) = extract_class_after_call(txt, kind) {
            info.relationships.push(Relationship {
                kind: kind.to_string(),
                target,
            });
        }
        return;
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        find_relationship_in_expr(child, source, info);
    }
}

fn extract_class_after_call(blob: &str, kind: &str) -> Option<String> {
    let needle = format!("->{kind}(");
    let idx = blob.find(&needle)? + needle.len();
    let rest = blob.get(idx..)?;
    let rest = rest.trim_start();
    if rest.starts_with('\\') {
        let end = rest.find("::class").or_else(|| rest.find(','))?;
        let full = rest[..end].trim();
        return Some(segment_class_name(full));
    }
    let end = rest.find("::class")?;
    let full = rest[..end].trim();
    Some(segment_class_name(full))
}

fn segment_class_name(fqn: &str) -> String {
    fqn.rsplit('\\').next().unwrap_or(fqn).to_string()
}

// ── Routes: `Route::verb('uri', [Ctl::class, 'action'])` ─────────────────────

pub(super) fn routes_from_tree(content: &str) -> Vec<RouteInfo> {
    let Some(tree) = parse_php(content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_routes(tree.root_node(), content, &mut out);
    out
}

fn collect_routes(node: Node<'_>, source: &str, out: &mut Vec<RouteInfo>) {
    if let Some(r) = route_from_call_node(node, source) {
        out.push(r);
    }
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        collect_routes(child, source, out);
    }
}

fn route_from_call_node(node: Node<'_>, source: &str) -> Option<RouteInfo> {
    let t = node_text(node, source)?.trim();
    let verb = LRE.route_verb.captures(t)?.get(1)?.as_str();
    let args = extract_call_arguments_blob(node, source)?;
    let uri = first_string_in_blob(&args)?;
    let (controller, action) = parse_route_array_target(&args)?;
    let name = extract_chained_route_name(t);
    Some(RouteInfo {
        verb: verb.to_ascii_uppercase(),
        uri,
        controller,
        action,
        name,
    })
}

fn extract_call_arguments_blob(node: Node<'_>, source: &str) -> Option<String> {
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if child.kind() == "arguments" {
            return node_text(child, source).map(|s| s.to_string());
        }
    }
    None
}

fn first_string_in_blob(args: &str) -> Option<String> {
    let s = args.trim().trim_start_matches('(').trim_end_matches(')');
    let s = s.trim();
    let q = s.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find(q)?;
    Some(rest[..end].to_string())
}

fn parse_route_array_target(args_blob: &str) -> Option<(String, String)> {
    let open = args_blob.find('[')?;
    let close = args_blob.rfind(']')?;
    let inner = args_blob[open + 1..close].trim();
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    let ctl = parts[0].trim().strip_suffix("::class")?;
    let ctl = segment_class_name(ctl.trim_start_matches('\\'));
    let act_raw = parts[1].trim();
    let act = act_raw
        .strip_prefix('\'')
        .or_else(|| act_raw.strip_prefix('"'))?
        .trim_end_matches('\'')
        .trim_end_matches('"');
    Some((ctl.to_string(), act.to_string()))
}

fn extract_chained_route_name(full_call_text: &str) -> Option<String> {
    LRE.route_name
        .captures(full_call_text)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_schema_create_kind_in_sample_migration() {
        let src = "<?php\nSchema::create('users', function (Blueprint $table) {\n});\n";
        let info = migration_from_tree(src).expect("migration");
        assert_eq!(info.table, "users");
    }
}
