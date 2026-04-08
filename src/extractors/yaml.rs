//! Generic YAML symbol extraction plus Ansible task shapes when the path matches
//! conventional Ansible layout ([`super::classifier::path_suggests_ansible_layout`]).
use anyhow::Result;
use tree_sitter::Node;

use crate::model::SymbolKind;

use super::classifier::path_suggests_ansible_layout;
use super::common::{node_text, parse_tree, push_symbol};

pub struct YamlExtractor;

impl super::LanguageExtractor for YamlExtractor {
    fn language_name(&self) -> &'static str {
        "yaml"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["yaml", "yml"]
    }

    fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<crate::model::StoredSymbol>> {
        let tree = parse_tree(tree_sitter_yaml::LANGUAGE, source)?;
        let mut symbols = Vec::new();
        let is_ansible = path_suggests_ansible_layout(std::path::Path::new(file_path));
        visit_stream(
            tree.root_node(),
            file_path,
            source,
            is_ansible,
            &[],
            &mut symbols,
        )?;
        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

fn visit_stream(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    is_ansible: bool,
    scopes: &[String],
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "document" | "stream" => {
                visit_stream(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            "block_node" => {
                visit_block_node(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            "block_mapping" => {
                visit_mapping(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            "block_sequence" if is_ansible => {
                visit_ansible_sequence(child, file_path, source, scopes, symbols)?;
            }
            _ => {
                // Recurse for unknown containers
                visit_stream(child, file_path, source, is_ansible, scopes, symbols)?;
            }
        }
    }
    Ok(())
}

fn visit_block_node(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    is_ansible: bool,
    scopes: &[String],
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block_mapping" => {
                visit_mapping(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            "block_sequence" if is_ansible => {
                visit_ansible_sequence(child, file_path, source, scopes, symbols)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn visit_mapping(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    is_ansible: bool,
    scopes: &[String],
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block_mapping_pair"
            && let Some(key_node) = child.child_by_field_name("key")
        {
            let key = extract_scalar_text(key_node, source);
            if key.is_empty() {
                continue;
            }

            let value_node = child.child_by_field_name("value");
            let has_nested = value_node.map(|v| has_block_mapping(v)).unwrap_or(false);

            let kind = if has_nested {
                SymbolKind::Type
            } else {
                SymbolKind::Constant
            };

            push_symbol(
                symbols, file_path, "yaml", scopes, &key, kind, child, source, None,
            )?;

            // Recurse into nested mappings with scope
            if has_nested && let Some(val) = value_node {
                let mut new_scopes = scopes.to_vec();
                new_scopes.push(key);
                visit_nested_value(val, file_path, source, is_ansible, &new_scopes, symbols)?;
            }
        }
    }
    Ok(())
}

fn visit_nested_value(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    is_ansible: bool,
    scopes: &[String],
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block_mapping" => {
                visit_mapping(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            "block_node" => {
                visit_nested_value(child, file_path, source, is_ansible, scopes, symbols)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn has_block_mapping(node: Node<'_>) -> bool {
    if node.kind() == "block_mapping" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if has_block_mapping(child) {
            return true;
        }
    }
    false
}

fn extract_scalar_text(node: Node<'_>, source: &str) -> String {
    // The key can be a flow_node > plain_scalar > string_scalar, or directly a scalar
    match node.kind() {
        "flow_node" | "block_node" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let text = extract_scalar_text(child, source);
                if !text.is_empty() {
                    return text;
                }
            }
            node_text(node, source)
                .unwrap_or_default()
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        }
        _ => node_text(node, source)
            .unwrap_or_default()
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string(),
    }
}

fn visit_ansible_sequence(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    scopes: &[String],
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    // Check whether any item in the sequence has a `hosts:` key — if so this
    // is a playbook; otherwise it is a standalone tasks file.
    let has_any_play = sequence_has_play(node, source);

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block_sequence_item" {
            let mut inner_cursor = child.walk();
            for inner in child.named_children(&mut inner_cursor) {
                if let Some(mapping) = find_block_mapping_in(inner) {
                    let has_hosts = has_yaml_key(mapping, source, "hosts");

                    if has_hosts {
                        // Play-level item
                        if let Some(name) = find_yaml_key_value(mapping, source, "name") {
                            push_symbol(
                                symbols,
                                file_path,
                                "ansible",
                                scopes,
                                &name,
                                SymbolKind::Class,
                                child,
                                source,
                                None,
                            )?;
                            extract_ansible_tasks(mapping, file_path, source, &name, symbols)?;
                        }
                    } else if !has_any_play {
                        // Standalone task file — use "task.tasks" as parent scope
                        let task_name = find_yaml_key_value(mapping, source, "name")
                            .or_else(|| find_module_fqcn(mapping, source));
                        if let Some(name) = task_name {
                            push_symbol(
                                symbols,
                                file_path,
                                "yaml",
                                &["task.tasks".to_string()],
                                &name,
                                SymbolKind::Function,
                                child,
                                source,
                                None,
                            )?;
                        }
                    } else {
                        // Non-play item inside a playbook sequence (e.g. pre_tasks at top level)
                        if let Some(name) = find_yaml_key_value(mapping, source, "name") {
                            push_symbol(
                                symbols,
                                file_path,
                                "yaml",
                                scopes,
                                &name,
                                SymbolKind::Function,
                                child,
                                source,
                                None,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Returns `true` if any item in the block sequence contains a `hosts:` key.
fn sequence_has_play(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block_sequence_item" {
            let mut inner_cursor = child.walk();
            for inner in child.named_children(&mut inner_cursor) {
                if let Some(mapping) = find_block_mapping_in(inner)
                    && has_yaml_key(mapping, source, "hosts")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Find the first mapping key that looks like a module FQCN (contains dots
/// and is not a well-known Ansible directive).
fn find_module_fqcn(mapping: Node<'_>, source: &str) -> Option<String> {
    static ANSIBLE_DIRECTIVES: &[&str] = &[
        "name",
        "become",
        "register",
        "when",
        "tags",
        "notify",
        "with_items",
        "loop",
        "vars",
        "environment",
        "ignore_errors",
        "changed_when",
        "failed_when",
        "delegate_to",
        "run_once",
        "no_log",
        "retries",
        "delay",
        "until",
        "block",
        "rescue",
        "always",
        "listen",
    ];
    let mut cursor = mapping.walk();
    for child in mapping.named_children(&mut cursor) {
        if child.kind() == "block_mapping_pair"
            && let Some(key) = child.child_by_field_name("key")
        {
            let text = extract_scalar_text(key, source);
            if text.contains('.') && !ANSIBLE_DIRECTIVES.contains(&text.as_str()) {
                return Some(text);
            }
        }
    }
    None
}

fn find_block_mapping_in(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "block_mapping" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(m) = find_block_mapping_in(child) {
            return Some(m);
        }
    }
    None
}

fn find_yaml_key_value(mapping: Node<'_>, source: &str, key_name: &str) -> Option<String> {
    let mut cursor = mapping.walk();
    for child in mapping.named_children(&mut cursor) {
        if child.kind() == "block_mapping_pair"
            && let Some(key) = child.child_by_field_name("key")
        {
            let text = extract_scalar_text(key, source);
            if text == key_name
                && let Some(val) = child.child_by_field_name("value")
            {
                return Some(extract_scalar_text(val, source));
            }
        }
    }
    None
}

fn has_yaml_key(mapping: Node<'_>, source: &str, key_name: &str) -> bool {
    find_yaml_key_value(mapping, source, key_name).is_some()
}

fn extract_ansible_tasks(
    play_mapping: Node<'_>,
    file_path: &str,
    source: &str,
    play_name: &str,
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = play_mapping.walk();
    for child in play_mapping.named_children(&mut cursor) {
        if child.kind() == "block_mapping_pair"
            && let Some(key) = child.child_by_field_name("key")
        {
            let text = extract_scalar_text(key, source);
            if (text == "tasks" || text == "handlers")
                && let Some(val) = child.child_by_field_name("value")
            {
                let parent_scope = format!("{play_name}.{text}");
                extract_task_list(val, file_path, source, &parent_scope, symbols)?;
            }
        }
    }
    Ok(())
}

fn extract_task_list(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    parent: &str,
    symbols: &mut Vec<crate::model::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block_sequence" => {
                extract_task_list(child, file_path, source, parent, symbols)?;
            }
            "block_sequence_item" => {
                let mut inner_cursor = child.walk();
                for inner in child.named_children(&mut inner_cursor) {
                    if let Some(mapping) = find_block_mapping_in(inner) {
                        let task_name = find_yaml_key_value(mapping, source, "name")
                            .or_else(|| find_module_fqcn(mapping, source));
                        if let Some(name) = task_name {
                            push_symbol(
                                symbols,
                                file_path,
                                "yaml",
                                &[parent.to_string()],
                                &name,
                                SymbolKind::Function,
                                child,
                                source,
                                None,
                            )?;
                        }
                    }
                }
            }
            "block_node" => {
                extract_task_list(child, file_path, source, parent, symbols)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::YamlExtractor;
    use crate::extractors::LanguageExtractor;

    #[test]
    fn yaml_extracts_top_level_keys() {
        let src = "services:\n  api:\n    image: myapp:latest\n    port: 8080\nfeature_flags:\n  signup: true\n";
        let s = YamlExtractor.extract("config.yaml", src).expect("yaml");
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert!(names.contains(&"services"), "missing services: {names:?}");
        assert!(
            names.contains(&"feature_flags"),
            "missing feature_flags: {names:?}"
        );
    }

    #[test]
    fn yaml_empty_file() {
        let s = YamlExtractor.extract("empty.yaml", "").expect("yaml");
        assert!(s.is_empty());
    }

    #[test]
    fn yaml_multi_document() {
        let src = "---\nfoo: bar\n---\nbaz: qux\n";
        let s = YamlExtractor.extract("multi.yaml", src).expect("yaml");
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert!(names.contains(&"foo"), "missing foo: {names:?}");
        assert!(names.contains(&"baz"), "missing baz: {names:?}");
    }
}
