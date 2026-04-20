//! **Vue single-file components**: `<script>`, `<template>`, and `<style>` blocks need SFC-aware
//! parsing, not one flat query pack over the whole file.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::core::pipeline::StoredSymbol;
use crate::core::symbols::{ExtractionTier, SymbolKind};

use crate::extractors::common::{first_line, node_text, parse_tree, qualify, source_slice};
use crate::extractors::containers::{
    RemapOptions, SegmentOrigin, extract_host_segment, find_html_tag_blocks,
};

pub struct VueExtractor;

impl crate::extractors::LanguageExtractor for VueExtractor {
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
            codesight_language_support::builtin_grammar("typescript")
                .expect("typescript grammar registered with language-support")
        } else {
            codesight_language_support::builtin_grammar("javascript")
                .expect("javascript grammar registered with language-support")
        };

        // Derive component name from filename (e.g. "Counter.vue" -> "Counter")
        let component_name = derive_component_name(file_path);

        let tree = parse_tree(grammar, block.content)?;
        let mut symbols = Vec::new();

        // Emit synthetic component class symbol spanning the entire .vue file.
        if let Some(ref comp_name) = component_name {
            symbols.push(StoredSymbol {
                id: format!("{file_path}::{comp_name}#class"),
                extraction_tier: ExtractionTier::ManualOverride,
                language: codesight_core::VUE,
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

        let delegate_language = if block.is_typescript {
            "typescript"
        } else {
            "javascript"
        };
        let mut delegated = extract_host_segment(
            delegate_language,
            file_path,
            source,
            block.content,
            SegmentOrigin::new(source, block.byte_offset),
            RemapOptions {
                language_override: Some("vue"),
                qualified_name_prefix: component_name.as_deref(),
                ..Default::default()
            },
        )?;
        overlay_script_comments(&mut delegated, &block);
        symbols.extend(delegated);

        // Check if this is an Options API export default object
        let root = tree.root_node();
        if detect_options_api(root, block.content) {
            extract_options_api(
                root,
                file_path,
                source,
                &block,
                component_name.as_deref(),
                &mut symbols,
            )?;
        }

        symbols.sort_by_key(|s| (s.start_byte, s.end_byte));
        symbols.dedup_by(|right, left| left.id == right.id);
        Ok(symbols)
    }
}

// ---------------------------------------------------------------------------
// Script block extraction
// ---------------------------------------------------------------------------

struct ScriptBlock<'a> {
    content: &'a str,
    /// 0-based line offset of the script content within the .vue file.
    line_offset: u32,
    /// Byte offset of the script content within the .vue file.
    byte_offset: usize,
    is_typescript: bool,
}

static LANG_TS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\blang\s*=\s*["']ts["']"#).expect("infallible: vue regex"));

fn extract_script_block(source: &str) -> Option<ScriptBlock<'_>> {
    let block = find_html_tag_blocks(source, "script")
        .ok()?
        .into_iter()
        .next()?;
    let (content, content_start_byte) =
        trim_leading_block_newline(block.content, block.content_start_byte);
    Some(ScriptBlock {
        content,
        line_offset: source[..content_start_byte].lines().count() as u32,
        byte_offset: content_start_byte,
        is_typescript: LANG_TS_RE.is_match(block.attrs),
    })
}

fn trim_leading_block_newline(content: &str, content_start_byte: usize) -> (&str, usize) {
    if let Some(trimmed) = content.strip_prefix("\r\n") {
        (trimmed, content_start_byte + 2)
    } else if let Some(trimmed) = content.strip_prefix('\n') {
        (trimmed, content_start_byte + 1)
    } else {
        (content, content_start_byte)
    }
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
    block: &ScriptBlock<'_>,
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
            let key_name = get_pair_key(&child, block.content);
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
                        block.content,
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
    block: &ScriptBlock<'_>,
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
                .and_then(|n| node_text(n, block.content).ok())
                .map(|s| s.to_string()),
            "pair" => {
                // Shorthand method or arrow in computed
                get_pair_key(&child, block.content)
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
                block.content,
                block,
                vue_source,
                None,
            )?;
        }
    }
    Ok(())
}

fn overlay_script_comments(symbols: &mut [StoredSymbol], block: &ScriptBlock<'_>) {
    for symbol in symbols {
        if symbol.summary != symbol.signature || symbol.start_byte < block.byte_offset {
            continue;
        }
        let local_start = symbol.start_byte - block.byte_offset;
        if let Some(doc) = extract_preceding_line_comment(block.content, local_start) {
            symbol.summary = doc;
        }
    }
}

fn extract_preceding_line_comment(script_source: &str, local_start: usize) -> Option<String> {
    let before = script_source.get(..local_start)?;
    let lines: Vec<&str> = before.lines().collect();
    let mut comments = Vec::new();
    for line in lines.iter().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if comments.is_empty() {
                continue;
            }
            break;
        }
        if let Some(comment) = trimmed.strip_prefix("//") {
            comments.push(comment.trim().to_string());
            continue;
        }
        break;
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

#[allow(clippy::too_many_arguments)]
fn push_vue_symbol(
    symbols: &mut Vec<StoredSymbol>,
    file_path: &str,
    scopes: &[String],
    name: &str,
    kind: SymbolKind,
    span: tree_sitter::Node<'_>,
    script_source: &str,
    block: &ScriptBlock<'_>,
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
    let start_line = span.start_position().row as u32 + 1 + block.line_offset;
    let end_line = span.end_position().row as u32 + 1 + block.line_offset;

    // Adjust byte offsets to .vue file coordinates.
    let start_byte = span.start_byte() + block.byte_offset;
    let end_byte = span.end_byte() + block.byte_offset;

    symbols.push(StoredSymbol {
        id: format!("{file_path}::{qualified_name}#{}", kind.as_str()),
        extraction_tier: ExtractionTier::ManualOverride,
        language: codesight_core::VUE,
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
    use crate::core::symbols::SymbolKind;
    use crate::extractors::LanguageExtractor;

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
        assert_eq!(greet.language, codesight_core::VUE);
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
        assert_eq!(iface.language, codesight_core::VUE);

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
        let funcs: std::collections::HashMap<&str, &crate::core::pipeline::StoredSymbol> = symbols
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
