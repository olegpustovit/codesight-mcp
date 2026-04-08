//! **Vue single-file components**: `<script>`, `<template>`, and `<style>` blocks need SFC-aware
//! parsing, not one flat `symbols.scm` over the whole file.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::model::{ExtractionTier, StoredSymbol, SymbolKind};

use super::common::{first_line, node_text, parse_tree, qualify, source_slice};

pub struct VueExtractor;

impl super::LanguageExtractor for VueExtractor {
    fn language_name(&self) -> &'static str {
        "vue"
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["vue"]
    }

    fn extract(&self, file_path: &str, source: &str) -> Result<Vec<StoredSymbol>> {
        let Some(block) = extract_script_block(source) else {
            return Ok(Vec::new());
        };

        let grammar: tree_sitter::Language = if block.is_typescript {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        } else {
            tree_sitter_javascript::LANGUAGE.into()
        };

        // Derive component name from filename (e.g. "Counter.vue" -> "Counter")
        let component_name = derive_component_name(file_path);

        let tree = parse_tree(grammar, &block.content)?;
        let mut symbols = Vec::new();

        // Emit synthetic component class symbol spanning the entire .vue file.
        if let Some(ref comp_name) = component_name {
            symbols.push(StoredSymbol {
                id: format!("{file_path}::{comp_name}#class"),
                extraction_tier: ExtractionTier::ManualOverride,
                language: "vue".to_string(),
                file_path: file_path.to_string(),
                qualified_name: comp_name.clone(),
                name: comp_name.clone(),
                kind: SymbolKind::Class,
                signature: format!("<script>{comp_name}"),
                summary: format!("<script>{comp_name}"),
                start_line: 1,
                end_line: source.lines().count().max(1) as u32,
                start_byte: 0,
                end_byte: source.len(),
                source_hash: blake3::hash(source.as_bytes()).to_hex().to_string(),
                source_text: source.to_string(),
                file_size_bytes: source.len() as u64,
            });
        }

        // Check if this is an Options API export default object
        let root = tree.root_node();
        let is_options_api = detect_options_api(root, &block.content);

        if is_options_api {
            extract_options_api(
                root,
                file_path,
                source,
                &block,
                component_name.as_deref(),
                &mut symbols,
            )?;
        } else {
            // Composition API / regular script: extract with standard JS/TS logic
            extract_composition_api(
                root,
                file_path,
                source,
                &block,
                component_name.as_deref(),
                &mut symbols,
            )?;
        }

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        Ok(symbols)
    }
}

// ---------------------------------------------------------------------------
// Script block extraction
// ---------------------------------------------------------------------------

struct ScriptBlock {
    content: String,
    /// 0-based line offset of the script content within the .vue file.
    line_offset: usize,
    /// Byte offset of the script content within the .vue file.
    byte_offset: usize,
    is_typescript: bool,
}

static SCRIPT_OPEN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<script\b([^>]*)>"#).unwrap());
static SCRIPT_CLOSE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)</script\s*>"#).unwrap());
static LANG_TS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\blang\s*=\s*["']ts["']"#).unwrap());

fn extract_script_block(source: &str) -> Option<ScriptBlock> {
    let open_match = SCRIPT_OPEN_RE.find(source)?;
    let attrs = SCRIPT_OPEN_RE.captures(source)?.get(1)?.as_str();
    let is_typescript = LANG_TS_RE.is_match(attrs);

    let content_start = open_match.end();
    let close_match = SCRIPT_CLOSE_RE.find(&source[content_start..])?;
    let content_end = content_start + close_match.start();

    let content = source[content_start..content_end].to_string();
    let line_offset = source[..content_start].matches('\n').count();
    let byte_offset = content_start;

    Some(ScriptBlock {
        content,
        line_offset,
        byte_offset,
        is_typescript,
    })
}

fn derive_component_name(file_path: &str) -> Option<String> {
    let path = std::path::Path::new(file_path);
    let stem = path.file_stem()?.to_str()?;
    Some(stem.to_string())
}

// ---------------------------------------------------------------------------
// Options API extraction
// ---------------------------------------------------------------------------

/// Detect if the script contains `export default { ... }` (Options API pattern).
fn detect_options_api(root: tree_sitter::Node<'_>, _source: &str) -> bool {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "export_statement" {
            // Look for a default export of an object
            let mut inner = child.walk();
            for grandchild in child.named_children(&mut inner) {
                if grandchild.kind() == "object" {
                    return true;
                }
            }
        }
    }
    false
}

fn extract_options_api(
    root: tree_sitter::Node<'_>,
    file_path: &str,
    vue_source: &str,
    block: &ScriptBlock,
    component_name: Option<&str>,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    // Find the export default object
    let Some(obj_node) = find_export_default_object(root) else {
        return Ok(());
    };

    let scopes: Vec<String> = component_name.iter().map(|s| s.to_string()).collect();

    // Walk the object's properties for methods, computed, props
    let mut cursor = obj_node.walk();
    for child in obj_node.named_children(&mut cursor) {
        if child.kind() == "pair" || child.kind() == "method_definition" {
            let key_name = get_pair_key(&child, &block.content);
            let Some(key) = key_name else { continue };

            match key.as_str() {
                "methods" | "computed" => {
                    // Extract method/computed entries as method symbols
                    if let Some(val) = get_pair_value(&child) {
                        extract_object_methods(
                            val, file_path, vue_source, block, &scopes, symbols,
                        )?;
                    }
                }
                "props" => {
                    // props captured as a constant
                    push_vue_symbol(
                        symbols,
                        file_path,
                        &scopes,
                        "props",
                        SymbolKind::Constant,
                        child,
                        &block.content,
                        block,
                        vue_source,
                        None,
                    )?;
                }
                "data" if child.kind() == "method_definition" => {
                    // data() method — skip or treat as method
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn find_export_default_object(root: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == "export_statement" {
            let mut inner = child.walk();
            for grandchild in child.named_children(&mut inner) {
                if grandchild.kind() == "object" {
                    return Some(grandchild);
                }
            }
        }
    }
    None
}

fn get_pair_key(node: &tree_sitter::Node<'_>, source: &str) -> Option<String> {
    if node.kind() == "method_definition" {
        let name_node = node.child_by_field_name("name")?;
        return node_text(name_node, source).ok().map(|s| s.to_string());
    }
    let key_node = node.child_by_field_name("key")?;
    node_text(key_node, source).ok().map(|s| s.to_string())
}

fn get_pair_value<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    node.child_by_field_name("value")
}

fn extract_object_methods(
    obj_node: tree_sitter::Node<'_>,
    file_path: &str,
    vue_source: &str,
    block: &ScriptBlock,
    scopes: &[String],
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    if obj_node.kind() != "object" {
        return Ok(());
    }
    let mut cursor = obj_node.walk();
    for child in obj_node.named_children(&mut cursor) {
        let name = match child.kind() {
            "method_definition" => child
                .child_by_field_name("name")
                .and_then(|n| node_text(n, &block.content).ok())
                .map(|s| s.to_string()),
            "pair" => {
                // Shorthand method or arrow in computed
                get_pair_key(&child, &block.content)
            }
            _ => None,
        };
        if let Some(name) = name {
            push_vue_symbol(
                symbols,
                file_path,
                scopes,
                &name,
                SymbolKind::Method,
                child,
                &block.content,
                block,
                vue_source,
                None,
            )?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Composition API extraction
// ---------------------------------------------------------------------------

fn extract_composition_api(
    root: tree_sitter::Node<'_>,
    file_path: &str,
    vue_source: &str,
    block: &ScriptBlock,
    component_name: Option<&str>,
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    let scopes: Vec<String> = component_name.iter().map(|s| s.to_string()).collect();

    visit_composition_node(root, file_path, vue_source, block, &scopes, symbols)?;
    Ok(())
}

fn visit_composition_node(
    node: tree_sitter::Node<'_>,
    file_path: &str,
    vue_source: &str,
    block: &ScriptBlock,
    scopes: &[String],
    symbols: &mut Vec<StoredSymbol>,
) -> Result<()> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, &block.content)?.to_string();
                let doc = extract_preceding_comment(node, &block.content);
                push_vue_symbol(
                    symbols,
                    file_path,
                    scopes,
                    &name,
                    SymbolKind::Function,
                    node,
                    &block.content,
                    block,
                    vue_source,
                    doc,
                )?;
            }
            return Ok(());
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, &block.content)?.to_string();
                push_vue_symbol(
                    symbols,
                    file_path,
                    scopes,
                    &name,
                    SymbolKind::Class,
                    node,
                    &block.content,
                    block,
                    vue_source,
                    None,
                )?;
            }
            return Ok(());
        }
        "interface_declaration" | "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, &block.content)?.to_string();
                push_vue_symbol(
                    symbols,
                    file_path,
                    scopes,
                    &name,
                    SymbolKind::Type,
                    node,
                    &block.content,
                    block,
                    vue_source,
                    None,
                )?;
            }
            return Ok(());
        }
        "lexical_declaration" => {
            // Handle const/let declarations
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(name_node, &block.content)?.to_string();
                        let value = child.child_by_field_name("value");

                        let kind = classify_composition_value(&value, &block.content);
                        let doc = extract_preceding_comment(node, &block.content);

                        push_vue_symbol(
                            symbols,
                            file_path,
                            scopes,
                            &name,
                            kind,
                            node,
                            &block.content,
                            block,
                            vue_source,
                            doc,
                        )?;
                    }
                }
            }
            return Ok(());
        }
        "import_statement" => {
            // Skip imports
            return Ok(());
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_composition_node(child, file_path, vue_source, block, scopes, symbols)?;
    }
    Ok(())
}

/// Classify the value of a const declaration in Composition API context.
/// - `ref(...)`, `computed(...)`, `defineProps(...)`, `defineEmits(...)` → Constant
/// - arrow functions, function expressions → Function
/// - everything else → Constant
fn classify_composition_value(value: &Option<tree_sitter::Node<'_>>, _source: &str) -> SymbolKind {
    let Some(val) = value else {
        return SymbolKind::Constant;
    };
    match val.kind() {
        "arrow_function" | "function_expression" | "generator_function" => SymbolKind::Function,
        _ => SymbolKind::Constant,
    }
}

/// Extract preceding line comment(s) as a docstring.
fn extract_preceding_comment(node: tree_sitter::Node<'_>, source: &str) -> Option<String> {
    let mut prev = node.prev_named_sibling();
    let mut comments = Vec::new();
    while let Some(p) = prev {
        if p.kind() == "comment" {
            let text = node_text(p, source).ok()?;
            // Strip // prefix
            let cleaned = text
                .trim_start_matches("//")
                .trim_start_matches("/*")
                .trim_end_matches("*/")
                .trim();
            comments.push(cleaned.to_string());
        } else {
            break;
        }
        prev = p.prev_named_sibling();
    }
    if comments.is_empty() {
        return None;
    }
    comments.reverse();
    Some(comments.join(" "))
}

// ---------------------------------------------------------------------------
// Symbol emission with line-offset adjustment
// ---------------------------------------------------------------------------

fn push_vue_symbol(
    symbols: &mut Vec<StoredSymbol>,
    file_path: &str,
    scopes: &[String],
    name: &str,
    kind: SymbolKind,
    span: tree_sitter::Node<'_>,
    script_source: &str,
    block: &ScriptBlock,
    vue_source: &str,
    doc: Option<String>,
) -> Result<()> {
    let qualified_name = qualify(scopes, name);
    let signature = first_line(span, script_source)?;
    let summary = doc.unwrap_or_else(|| signature.clone());
    let script_text = source_slice(span, script_source)?.to_string();
    let source_hash = blake3::hash(script_text.as_bytes()).to_hex().to_string();

    // Adjust line numbers: tree-sitter rows are 0-based in the script block,
    // add the line offset to get the correct position in the .vue file.
    let start_line = (span.start_position().row + 1 + block.line_offset) as u32;
    let end_line = (span.end_position().row + 1 + block.line_offset) as u32;

    // Adjust byte offsets to .vue file coordinates.
    let start_byte = span.start_byte() + block.byte_offset;
    let end_byte = span.end_byte() + block.byte_offset;

    symbols.push(StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: "vue".to_string(),
        file_path: file_path.to_string(),
        qualified_name,
        name: name.to_string(),
        kind,
        signature,
        summary,
        start_line,
        end_line,
        start_byte,
        end_byte,
        source_hash,
        source_text: script_text,
        file_size_bytes: vue_source.len() as u64,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::VueExtractor;
    use crate::extractors::LanguageExtractor;
    use crate::model::SymbolKind;

    const VUE_JS_SOURCE: &str = r#"<template>
  <div>{{ message }}</div>
</template>

<script setup>
import { ref } from 'vue'

// Greet a user
function greet(name) {
  return `Hello, ${name}`
}

class MyComponent {
  mounted() {}
}
</script>
"#;

    const VUE_TS_SOURCE: &str = r#"<template><div /></template>
<script setup lang="ts">
import { ref } from 'vue'

interface User { name: string }

// Get the user name
function getName(user: User): string {
  return user.name
}
</script>
"#;

    const VUE_COMPOSITION_SOURCE: &str = r#"<script setup lang="ts">
import { ref, computed } from 'vue'

// Current count
const count = ref(0)
const doubled = computed(() => count.value * 2)
const props = defineProps<{ title: string; max?: number }>()
const emit = defineEmits(['update'])

function increment() {
  count.value++
  emit('update', count.value)
}

function reset() {
  count.value = 0
}
</script>
<template><div>{{ count }}</div></template>
"#;

    const VUE_OPTIONS_SOURCE: &str = r#"<script>
export default {
  props: { items: Array, loading: Boolean },
  data() { return { selected: null } },
  computed: {
    count() { return this.items.length },
    hasItems() { return this.count > 0 }
  },
  methods: {
    select(item) { this.selected = item },
    clear() { this.selected = null }
  }
}
</script>
<template><div/></template>
"#;

    #[test]
    fn vue_parse_js() {
        let symbols = VueExtractor.extract("App.vue", VUE_JS_SOURCE).unwrap();

        let greet = symbols
            .iter()
            .find(|s| s.name == "greet")
            .expect("missing greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(greet.language, "vue");
        assert!(
            greet.summary.contains("Greet a user"),
            "summary: {}",
            greet.summary
        );
        assert_eq!(
            greet.start_line, 9,
            "greet should be on line 9 of .vue file"
        );

        let cls = symbols
            .iter()
            .find(|s| s.name == "MyComponent")
            .expect("missing MyComponent");
        assert_eq!(cls.kind, SymbolKind::Class);
    }

    #[test]
    fn vue_parse_ts() {
        let symbols = VueExtractor.extract("User.vue", VUE_TS_SOURCE).unwrap();

        let iface = symbols
            .iter()
            .find(|s| s.name == "User" && s.kind == SymbolKind::Type)
            .expect("missing User interface");
        assert_eq!(iface.language, "vue");

        let func = symbols
            .iter()
            .find(|s| s.name == "getName")
            .expect("missing getName");
        assert_eq!(func.kind, SymbolKind::Function);
        assert!(func.signature.contains("string"), "sig: {}", func.signature);
        assert!(
            func.summary.contains("Get the user name"),
            "summary: {}",
            func.summary
        );
    }

    #[test]
    fn vue_parse_no_script() {
        let source = "<template><div>hello</div></template>";
        let symbols = VueExtractor.extract("Static.vue", source).unwrap();
        assert!(symbols.is_empty());
    }

    #[test]
    fn vue_composition_component_symbol() {
        let symbols = VueExtractor
            .extract("Counter.vue", VUE_COMPOSITION_SOURCE)
            .unwrap();
        let comp: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "Counter")
            .collect();
        assert_eq!(comp.len(), 1);
        assert_eq!(comp[0].start_line, 1);
    }

    #[test]
    fn vue_composition_ref_captured() {
        let symbols = VueExtractor
            .extract("Counter.vue", VUE_COMPOSITION_SOURCE)
            .unwrap();
        let names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"count"), "missing count: {names:?}");
        assert!(names.contains(&"doubled"), "missing doubled: {names:?}");
    }

    #[test]
    fn vue_composition_define_macros() {
        let symbols = VueExtractor
            .extract("Counter.vue", VUE_COMPOSITION_SOURCE)
            .unwrap();
        let names: Vec<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"props"), "missing props: {names:?}");
        assert!(names.contains(&"emit"), "missing emit: {names:?}");
    }

    #[test]
    fn vue_composition_functions() {
        let symbols = VueExtractor
            .extract("Counter.vue", VUE_COMPOSITION_SOURCE)
            .unwrap();
        let funcs: std::collections::HashMap<&str, &crate::model::StoredSymbol> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .map(|s| (s.name.as_str(), s))
            .collect();
        assert!(funcs.contains_key("increment"), "missing increment");
        assert!(funcs.contains_key("reset"), "missing reset");
        assert!(
            funcs["increment"].start_line > 1,
            "increment should not be on line 1"
        );
    }

    #[test]
    fn vue_composition_parent_relationship() {
        let symbols = VueExtractor
            .extract("Counter.vue", VUE_COMPOSITION_SOURCE)
            .unwrap();
        let comp = symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Class)
            .expect("no class");
        let children: Vec<_> = symbols
            .iter()
            .filter(|s| s.qualified_name.starts_with(&format!("{}.", comp.name)) && s.id != comp.id)
            .collect();
        assert!(
            children.len() >= 4,
            "expected >= 4 children under component, got {}: {:?}",
            children.len(),
            children.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn vue_options_methods() {
        let symbols = VueExtractor
            .extract("MyList.vue", VUE_OPTIONS_SOURCE)
            .unwrap();
        let methods: std::collections::HashSet<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(methods.contains("select"), "missing select: {methods:?}");
        assert!(methods.contains("clear"), "missing clear: {methods:?}");
    }

    #[test]
    fn vue_options_computed() {
        let symbols = VueExtractor
            .extract("MyList.vue", VUE_OPTIONS_SOURCE)
            .unwrap();
        let methods: std::collections::HashSet<&str> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(methods.contains("count"), "missing count: {methods:?}");
        assert!(
            methods.contains("hasItems"),
            "missing hasItems: {methods:?}"
        );
    }

    #[test]
    fn vue_options_props() {
        let symbols = VueExtractor
            .extract("MyList.vue", VUE_OPTIONS_SOURCE)
            .unwrap();
        let props: Vec<_> = symbols
            .iter()
            .filter(|s| s.name == "props" && s.kind == SymbolKind::Constant)
            .collect();
        assert_eq!(props.len(), 1, "expected exactly 1 props constant");
    }

    #[test]
    fn vue_extension_mapping() {
        use crate::extractors::ExtractorRegistry;
        let registry = ExtractorRegistry::default();
        let ext = registry
            .extractor_for_path(std::path::Path::new("src/components/Counter.vue"))
            .expect("no extractor for .vue");
        assert_eq!(ext.language_name(), "vue");
    }
}
