use super::*;

#[test]
fn root_keys_decode_once_preserve_pair_spans_and_exclude_nested_keys() {
    // Small generated reproducer: repeated escapes split string_content nodes.
    let long_key = "segment\\\"".repeat(8);
    let source = format!(
        r#"{{"ordinary":1,"quote\"key":2,"slash\\key":3,"\u03b2":4,"":5,{}:6,"nested":{{"not-root":7}}}}"#,
        serde_json::to_string(&long_key).unwrap()
    );
    let expected: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&source).unwrap();
    let symbols = JsonSupport.extract_symbols("sample.json", &source).unwrap();
    assert_eq!(
        symbols.len(),
        expected.len(),
        "each root pair must yield exactly one symbol"
    );
    let actual: std::collections::BTreeSet<_> =
        symbols.iter().map(|symbol| symbol.name.as_str()).collect();
    assert_eq!(actual, expected.keys().map(String::as_str).collect());
    for symbol in symbols {
        assert_eq!(symbol.qualified_name, symbol.name);
        assert_eq!(symbol.id, format!("sample.json::{}#constant", symbol.name));
        assert_eq!(
            &source[symbol.start_byte..symbol.end_byte],
            symbol.source_text
        );
        let pair: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&format!("{{{}}}", symbol.source_text)).unwrap();
        assert_eq!(pair.len(), 1);
        assert_eq!(pair.get(&symbol.name), expected.get(&symbol.name));
    }
}

#[test]
fn long_escaped_root_key_produces_one_complete_symbol() {
    let key = "fragment\\\"".repeat(512);
    let source = format!("{{{}:0.5}}", serde_json::to_string(&key).unwrap());
    let symbols = JsonSupport.extract_symbols("long.json", &source).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, key);
    assert_eq!(
        &source[symbols[0].start_byte..symbols[0].end_byte],
        symbols[0].source_text
    );
}

#[test]
fn custom_json_query_retains_its_scope_when_decoding_the_leaf_name() {
    let extractor = crate::scm_engine::ScmExtractor::new(
        &crate::scm_engine::ScmMetadata {
            name: "json".into(),
            extensions: vec!["json".into()],
            grammar: "json".into(),
            override_extractor: None,
        },
        r#"(document (object (pair key: (string (string_content) @scope)
            value: (object (pair key: (string) @name) @definition.constant))))"#,
    )
    .unwrap();
    let source = r#"{"Outer":{"quote\"key":1}}"#;
    let symbols = extractor.extract("custom.json", source).unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "quote\"key");
    assert_eq!(symbols[0].qualified_name, "Outer.quote\"key");
    assert_eq!(symbols[0].id, "custom.json::Outer.quote\"key#constant");
    assert_eq!(
        &source[symbols[0].start_byte..symbols[0].end_byte],
        symbols[0].source_text
    );
}
