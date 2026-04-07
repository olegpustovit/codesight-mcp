use tree_sitter::{Language, Parser, Query};

fn dump_tree(node: &tree_sitter::Node, source: &str, indent: usize) {
    if node.is_named() {
        let text: String = source[node.start_byte()..node.end_byte()]
            .chars()
            .take(50)
            .collect();
        let text = text.replace('\n', "\\n");
        let field = node.parent().and_then(|p| {
            (0..p.child_count()).find_map(|i| {
                if p.child(i as u32).map(|c| c.id()) == Some(node.id()) {
                    p.field_name_for_child(i as u32)
                } else {
                    None
                }
            })
        });
        let f = field.map(|f| format!(" [{f}]")).unwrap_or_default();
        println!(
            "{:indent$}({}) L{}..L{}{f} «{text}»",
            "",
            node.kind(),
            node.start_position().row,
            node.end_position().row,
            indent = indent
        );
    }
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            dump_tree(&c.node(), source, indent + 2);
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
}

fn try_query(lang: &Language, name: &str, query_src: &str) {
    match Query::new(lang, query_src) {
        Ok(q) => println!(
            "  [{name}] OK ({} patterns, {} captures)",
            q.pattern_count(),
            q.capture_names().len()
        ),
        Err(e) => println!("  [{name}] FAILED: {e}"),
    }
}

fn main() {
    // --- DART ---
    println!("=== DART AST ===");
    let dart_src = r#"
class Calculator {
  int add(int a, int b) => a + b;
  bool get isPositive => true;
}
void greet(String name) {}
enum Status { active, inactive }
typedef JsonMap = Map<String, dynamic>;
mixin Scrollable {}
extension StringExt on String {}
"#;
    let lang_dart: Language = tree_sitter_dart::LANGUAGE.into();
    let mut p = Parser::new();
    p.set_language(&lang_dart).unwrap();
    let tree = p.parse(dart_src, None).unwrap();
    dump_tree(&tree.root_node(), dart_src, 0);

    println!("\n=== DART QUERY TESTS ===");
    try_query(
        &lang_dart,
        "class_declaration",
        "(class_declaration name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "class_definition",
        "(class_definition name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "enum_declaration",
        "(enum_declaration name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "mixin_declaration",
        "(mixin_declaration name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "function_signature",
        "(function_signature name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "method_signature",
        "(method_signature (function_signature name: (identifier) @n)) @d",
    );
    try_query(
        &lang_dart,
        "type_alias",
        "(type_alias (type_identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "extension_declaration",
        "(extension_declaration name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "extension_type_declaration",
        "(extension_type_declaration name: (identifier) @n) @d",
    );
    try_query(
        &lang_dart,
        "getter_signature",
        "(getter_signature name: (identifier) @n) @d",
    );

    // --- XML ---
    println!("\n=== XML AST ===");
    let xml_src = r#"<?xml version="1.0"?>
<config>
  <database>
    <host>localhost</host>
  </database>
</config>"#;
    let lang_xml: Language = tree_sitter_xml::LANGUAGE_XML.into();
    let mut p = Parser::new();
    p.set_language(&lang_xml).unwrap();
    let tree = p.parse(xml_src, None).unwrap();
    dump_tree(&tree.root_node(), xml_src, 0);

    println!("\n=== XML QUERY TESTS ===");
    try_query(&lang_xml, "STag+Name", "(element (STag (Name) @n)) @d");
    try_query(
        &lang_xml,
        "start_tag+tag_name",
        "(element (start_tag (tag_name) @n)) @d",
    );
    try_query(
        &lang_xml,
        "StartTag+Name",
        "(element (StartTag (Name) @n)) @d",
    );

    // --- NIX ---
    println!("\n=== NIX AST ===");
    let nix_src = r#"
let
  greet = name: "Hello ${name}";
  add = a: b: a + b;
  version = "1.0";
in { inherit greet add version; }
"#;
    let lang_nix: Language = tree_sitter_nix::LANGUAGE.into();
    let mut p = Parser::new();
    p.set_language(&lang_nix).unwrap();
    let tree = p.parse(nix_src, None).unwrap();
    dump_tree(&tree.root_node(), nix_src, 0);

    println!("\n=== NIX QUERY TESTS ===");
    try_query(
        &lang_nix,
        "binding+attrpath",
        "(binding attrpath: (attrpath attr: (identifier) @n)) @d",
    );
    try_query(
        &lang_nix,
        "bind+attrpath",
        "(bind (attrpath (attr_identifier) @n)) @d",
    );
    try_query(&lang_nix, "binding_plain", "(binding (attrpath) @n) @d");
}
