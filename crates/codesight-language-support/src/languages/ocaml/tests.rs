use std::path::Path;

#[test]
fn implementation_and_interface_extract_structural_definitions() {
    let registry = crate::builtin_registry();
    let samples = [
        (
            "sample.ml",
            "type number = int\nmodule M = struct\nlet inc x = x + 1\nlet twice = fun x -> inc (inc x)\nend\n",
            vec!["number", "M", "inc", "twice"],
        ),
        (
            "sample.mli",
            "type number\nval inc : int -> int\nmodule M : sig\n val twice : int -> int\nend\n",
            vec!["number", "inc", "M", "twice"],
        ),
    ];
    for (path, source, expected) in samples {
        let language = registry.for_path(Path::new(path)).unwrap();
        let tree =
            crate::extract_tree_sitter::parse_tree(language.grammar().unwrap(), source).unwrap();
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        let symbols = language.extract_symbols(path, source).unwrap();
        for name in expected {
            let symbol = symbols
                .iter()
                .find(|s| s.name == name)
                .unwrap_or_else(|| panic!("{path}: {name} missing: {symbols:?}"));
            assert_eq!(
                &source[symbol.start_byte..symbol.end_byte],
                symbol.source_text
            );
        }
        assert!(
            symbols
                .iter()
                .find(|s| s.name == "twice")
                .unwrap()
                .qualified_name
                .contains("M"),
            "{symbols:?}"
        );
        assert!(!crate::supports_import_extraction(language.id().as_str()));
    }
    assert!(registry.for_path(Path::new("program.rays")).is_none());
}

#[test]
fn ocaml_identifier_syntax_excludes_comments_and_strings() {
    let source = "(* inc *)\nlet inc x = x + 1\nlet use x = inc x\nlet text = \"inc\"\n";
    assert_eq!(
        crate::identifier_syntax::positions("sample.ml", source, "inc"),
        Some(vec![(2, 5), (3, 13)])
    );
}
