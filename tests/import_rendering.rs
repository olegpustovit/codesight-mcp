//! ROADMAP P1.6 — language-aware import rendering. Each language's
//! `display_import_lines` must emit source in that language's own syntax,
//! and the rendered lines must round-trip back through the language's
//! `extract_imports` parser to the same structured edges.

use std::path::Path;
use symslice_language_support::builtin_registry;
use symslice_language_support::imports_types::ImportEdge;

fn lang_for(path: &str) -> &'static dyn symslice_language_support::traits::Language {
    builtin_registry()
        .for_path(Path::new(path))
        .expect("language registered")
}

fn edge(spec: &str, names: &[&str]) -> ImportEdge {
    ImportEdge {
        specifier: spec.to_string(),
        names: names.iter().map(|s| s.to_string()).collect(),
    }
}

// ── Rust ─────────────────────────────────────────────────────────────────────

#[test]
fn rust_render_emits_use_syntax_not_esm() {
    let rust = lang_for("x.rs");
    let rendered = rust.display_import_lines(&[
        edge("serde_json", &["Value"]),
        edge("std::collections", &["FxHashMap", "FxHashSet"]),
        edge("std::fs", &[]),
    ]);
    assert_eq!(rendered[0], "use serde_json::Value;");
    assert_eq!(rendered[1], "use std::collections::{FxHashMap, FxHashSet};");
    assert_eq!(rendered[2], "use std::fs;");
    for line in &rendered {
        assert!(
            !line.contains("from"),
            "Rust rendering leaked ES-module syntax: {line}"
        );
    }
}

#[test]
fn rust_render_roundtrips_through_extract_imports() {
    let rust = lang_for("x.rs");
    let original = vec![
        edge("serde_json", &["Value"]),
        edge("std::collections", &["FxHashMap"]),
    ];
    let rendered = rust.display_import_lines(&original).join("\n");
    let parsed = rust.extract_imports(&rendered);
    // Parser may split `use spec::{a,b}` differently; at minimum, every
    // (specifier, name) pair from the original must show up in parsed.
    for orig in &original {
        for name in &orig.names {
            let found = parsed.iter().any(|e| {
                e.specifier.contains(&orig.specifier)
                    && (e.names.iter().any(|n| n == name) || e.specifier.ends_with(name))
            });
            assert!(
                found,
                "rendered line lost edge ({}, {}): rendered={rendered:?} parsed={parsed:?}",
                orig.specifier, name
            );
        }
    }
}

// ── Python ───────────────────────────────────────────────────────────────────

#[test]
fn python_render_emits_native_syntax() {
    let py = lang_for("x.py");
    let rendered = py.display_import_lines(&[
        edge("os", &[]),
        edge("collections", &["OrderedDict", "deque"]),
        edge("foo.bar", &["baz"]),
    ]);
    assert_eq!(rendered[0], "import os");
    assert_eq!(rendered[1], "from collections import OrderedDict, deque");
    assert_eq!(rendered[2], "from foo.bar import baz");
    for line in &rendered {
        assert!(
            !line.contains("{"),
            "python render leaked ESM braces: {line}"
        );
    }
}

#[test]
fn python_render_roundtrips_through_extract_imports() {
    let py = lang_for("x.py");
    let original = vec![
        edge("os", &[]),
        edge("collections", &["OrderedDict"]),
        edge("foo.bar", &["baz"]),
    ];
    let rendered = py.display_import_lines(&original).join("\n");
    let parsed = py.extract_imports(&rendered);
    for orig in &original {
        let hit = parsed.iter().any(|p| p.specifier == orig.specifier);
        assert!(
            hit,
            "python roundtrip lost specifier '{}': rendered={rendered:?} parsed={parsed:?}",
            orig.specifier
        );
    }
}

// ── Java / C# / Swift (render shape only — extract_imports coverage varies) ──

#[test]
fn java_render_emits_import_statements_with_semicolons() {
    let java = lang_for("X.java");
    let rendered = java.display_import_lines(&[
        edge("java.util", &["List", "Map"]),
        edge("java.io.IOException", &[]),
    ]);
    assert_eq!(rendered[0], "import java.util.List;");
    assert_eq!(rendered[1], "import java.util.Map;");
    assert_eq!(rendered[2], "import java.io.IOException;");
}

#[test]
fn csharp_render_emits_using_statements() {
    let cs = lang_for("X.cs");
    let rendered = cs.display_import_lines(&[
        edge("System.Collections.Generic", &["List"]),
        edge("System", &[]),
    ]);
    assert_eq!(rendered[0], "using System.Collections.Generic.List;");
    assert_eq!(rendered[1], "using System;");
}

#[test]
fn swift_render_emits_module_imports() {
    let swift = lang_for("x.swift");
    let rendered = swift.display_import_lines(&[edge("Foundation", &[]), edge("UIKit", &[])]);
    assert_eq!(rendered, vec!["import Foundation", "import UIKit"]);
}
