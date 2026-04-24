fn main() {
    let lang: tree_sitter::Language = tree_sitter_xml::LANGUAGE_XML.into();
    let query_src = include_str!("../src/languages/xml/queries/tags.scm");
    match tree_sitter::Query::new(&lang, query_src) {
        Ok(q) => println!(
            "XML query OK: {} patterns, {} captures",
            q.pattern_count(),
            q.capture_names().len()
        ),
        Err(e) => println!("XML query FAILED: {e}"),
    }
}
