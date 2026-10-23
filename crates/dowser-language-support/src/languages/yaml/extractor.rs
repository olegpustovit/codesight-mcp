//! Generic YAML symbols come from the shared query-pack engine. Ansible keeps a small
//! path-aware post-pass because plays/tasks are host semantics layered over YAML structure.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use tree_sitter::Node;

use dowser_core::symbols::{ExtractionTier, SymbolKind};

use crate::extract_tree_sitter::{node_text, parse_tree, push_symbol};
use crate::query_pack_catalog::QueryPackCatalog;
use crate::scm_engine::ScmExtractor;

pub struct YamlExtractor;

impl YamlExtractor {
    pub fn language_name(&self) -> &'static str {
        "yaml"
    }

    pub fn supported_extensions(&self) -> &'static [&'static str] {
        &["yaml", "yml"]
    }

    pub fn extraction_tier(&self) -> ExtractionTier {
        ExtractionTier::CuratedQueryPack
    }

    pub fn extract(
        &self,
        file_path: &str,
        source: &str,
    ) -> anyhow::Result<Vec<dowser_core::StoredSymbol>> {
        let resolved = crate::builtin_registry().classify_path(Path::new(file_path));
        let is_ansible = resolved.profile.is_some_and(|p| p.as_str() == "ansible");
        if !is_ansible {
            return yaml_pack()?.extract(file_path, source);
        }

        let tree = parse_tree(
            crate::builtin_grammar("yaml").expect("yaml grammar registered with language-support"),
            source,
        )?;
        let mut symbols = Vec::new();
        visit_ansible_nodes(tree.root_node(), file_path, source, &mut symbols)?;
        if symbols.is_empty() {
            return yaml_pack()?.extract(file_path, source);
        }
        symbols.sort_by_key(|symbol| (symbol.start_byte, symbol.end_byte));
        Ok(symbols)
    }
}

fn yaml_pack() -> anyhow::Result<&'static ScmExtractor> {
    // Built-in pack is compiled in; a missing entry is a logic bug, not a runtime error.
    static YAML_PACK: OnceLock<ScmExtractor> = OnceLock::new();
    Ok(YAML_PACK.get_or_init(|| {
        let catalog = QueryPackCatalog::builtin();
        let pack = catalog
            .find_by_pack_id("yaml")
            .expect("infallible: built-in catalog missing yaml query pack");
        ScmExtractor::from_pack(pack).expect("infallible: yaml pack failed to build")
    }))
}

fn visit_ansible_nodes(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    symbols: &mut Vec<dowser_core::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "stream" | "document" | "block_node" => {
                visit_ansible_nodes(child, file_path, source, symbols)?;
            }
            "block_sequence" => {
                visit_ansible_sequence(child, file_path, source, &[], symbols)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn visit_ansible_sequence(
    node: Node<'_>,
    file_path: &str,
    source: &str,
    scopes: &[String],
    symbols: &mut Vec<dowser_core::StoredSymbol>,
) -> Result<()> {
    let has_any_play = sequence_has_play(node, source);

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "block_sequence_item" {
            continue;
        }
        let mut inner_cursor = child.walk();
        for inner in child.named_children(&mut inner_cursor) {
            if let Some(mapping) = find_block_mapping_in(inner) {
                let has_hosts = has_yaml_key(mapping, source, "hosts");

                if has_hosts {
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
                } else if let Some(name) = find_yaml_key_value(mapping, source, "name") {
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
    Ok(())
}

fn sequence_has_play(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "block_sequence_item" {
            continue;
        }
        let mut inner_cursor = child.walk();
        for inner in child.named_children(&mut inner_cursor) {
            if let Some(mapping) = find_block_mapping_in(inner)
                && has_yaml_key(mapping, source, "hosts")
            {
                return true;
            }
        }
    }
    false
}

fn extract_scalar_text(node: Node<'_>, source: &str) -> String {
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
        if let Some(mapping) = find_block_mapping_in(child) {
            return Some(mapping);
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
                && let Some(value) = child.child_by_field_name("value")
            {
                return Some(extract_scalar_text(value, source));
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
    symbols: &mut Vec<dowser_core::StoredSymbol>,
) -> Result<()> {
    let mut cursor = play_mapping.walk();
    for child in play_mapping.named_children(&mut cursor) {
        if child.kind() == "block_mapping_pair"
            && let Some(key) = child.child_by_field_name("key")
        {
            let text = extract_scalar_text(key, source);
            if (text == "tasks" || text == "handlers")
                && let Some(value) = child.child_by_field_name("value")
            {
                let parent_scope = format!("{play_name}.{text}");
                extract_task_list(value, file_path, source, &parent_scope, symbols)?;
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
    symbols: &mut Vec<dowser_core::StoredSymbol>,
) -> Result<()> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block_sequence" | "block_node" => {
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
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::YamlExtractor;

    #[test]
    fn yaml_extracts_top_level_keys() {
        let src = "services:\n  api:\n    image: myapp:latest\n    port: 8080\nfeature_flags:\n  signup: true\n";
        let symbols = YamlExtractor.extract("config.yaml", src).expect("yaml");
        let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert!(names.contains(&"services"), "missing services: {names:?}");
        assert!(
            names.contains(&"feature_flags"),
            "missing feature_flags: {names:?}"
        );
    }

    #[test]
    fn yaml_empty_file() {
        let symbols = YamlExtractor.extract("empty.yaml", "").expect("yaml");
        assert!(symbols.is_empty());
    }

    #[test]
    fn yaml_multi_document() {
        let src = "---\nfoo: bar\n---\nbaz: qux\n";
        let symbols = YamlExtractor.extract("multi.yaml", src).expect("yaml");
        let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert!(names.contains(&"foo"), "missing foo: {names:?}");
        assert!(names.contains(&"baz"), "missing baz: {names:?}");
    }
}
