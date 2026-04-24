//! Vue has no tree-sitter grammar — its imports are the `<script>` block
//! parsed as JS plus any template components referenced as tags.

use std::collections::HashSet;

use crate::imports_types::ImportEdge;
use crate::languages::javascript::JavaScript;
use crate::traits::Language;

pub fn extract(content: &str) -> Vec<ImportEdge> {
    let mut edges = Vec::new();

    let script_content = extract_script_block(content);
    let mut script_edges = Vec::new();
    if let Some(ref script) = script_content {
        script_edges = JavaScript.extract_imports(script);
    }

    let mut imported_names: HashSet<String> = HashSet::new();
    for edge in &script_edges {
        for name in &edge.names {
            imported_names.insert(name.clone());
        }
        if let Some(stem) = edge.specifier.rsplit('/').next() {
            let stem = stem.split('.').next().unwrap_or(stem);
            imported_names.insert(stem.to_string());
        }
    }
    edges.extend(script_edges);

    if let Some(template) = extract_template_block(content) {
        let components = extract_template_components(&template);
        for component in components {
            if !imported_names.contains(&component) {
                edges.push(ImportEdge {
                    specifier: component,
                    names: Vec::new(),
                });
            }
        }
    }

    edges
}

const VUE_BUILTINS: &[&str] = &[
    "Transition",
    "TransitionGroup",
    "KeepAlive",
    "Teleport",
    "Suspense",
    "Component",
    "Slot",
];

const HTML_ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "param",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
    "svg",
    "path",
    "circle",
    "rect",
    "line",
    "polyline",
    "polygon",
    "text",
    "g",
    "defs",
    "use",
    "clipPath",
    "mask",
    "component",
    "transition",
    "transition-group",
    "keep-alive",
    "teleport",
    "suspense",
    "slot",
];

fn extract_script_block(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<script")?;
    let tag_end = content[start..].find('>')? + start + 1;
    let end = lower[tag_end..].find("</script>")?;
    Some(content[tag_end..tag_end + end].to_string())
}

fn extract_template_block(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let start = lower.find("<template")?;
    let tag_end = content[start..].find('>')? + start + 1;
    let end = lower[tag_end..].find("</template>")?;
    Some(content[tag_end..tag_end + end].to_string())
}

fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

fn extract_template_components(template: &str) -> Vec<String> {
    let mut components = HashSet::new();
    let html_set: HashSet<&str> = HTML_ELEMENTS.iter().copied().collect();
    let builtin_set: HashSet<&str> = VUE_BUILTINS.iter().copied().collect();

    let bytes = template.as_bytes();
    let len = bytes.len();
    let mut index = 0;

    while index < len {
        if bytes[index] == b'<' && index + 1 < len && bytes[index + 1] != b'/' {
            index += 1;
            while index < len && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            let start = index;
            while index < len
                && (bytes[index].is_ascii_alphanumeric()
                    || bytes[index] == b'-'
                    || bytes[index] == b'_')
            {
                index += 1;
            }
            if index > start {
                let tag = &template[start..index];
                let pascal = if tag.contains('-') {
                    kebab_to_pascal(tag)
                } else {
                    tag.to_string()
                };
                let tag_lower = tag.to_ascii_lowercase();
                if !html_set.contains(tag_lower.as_str())
                    && !builtin_set.contains(pascal.as_str())
                    && tag
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase() || tag.contains('-'))
                {
                    components.insert(pascal);
                }
            }
        } else {
            index += 1;
        }
    }

    let mut result: Vec<String> = components.into_iter().collect();
    result.sort();
    result
}
