//! Integration tests: dart imports.

use std::path::Path;

use codesight_mcp::extractors::ExtractorRegistry;
use codesight_imports::imports::extract_imports;

// ---------------------------------------------------------------------------
// Dart source fixture (inline, matching reference)
// ---------------------------------------------------------------------------

const DART_SOURCE: &str = "\
import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import './models/user.dart';
import '../utils/helpers.dart';
export 'src/api.dart';
";

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_dart_in_language_extractors() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("main.dart"));
    assert!(
        ext.is_some(),
        "Dart must be registered in ExtractorRegistry"
    );
    assert_eq!(ext.unwrap().language_name(), "dart");
}

#[test]
fn test_extracts_dart_stdlib_imports() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    let specifiers: Vec<&str> = edges.iter().map(|e| e.specifier.as_str()).collect();
    assert!(
        specifiers.contains(&"dart:async"),
        "should contain dart:async: {specifiers:?}"
    );
    assert!(
        specifiers.contains(&"dart:io"),
        "should contain dart:io: {specifiers:?}"
    );
}

#[test]
fn test_extracts_package_imports() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    let specifiers: Vec<&str> = edges.iter().map(|e| e.specifier.as_str()).collect();
    assert!(specifiers.contains(&"package:flutter/material.dart"));
    assert!(specifiers.contains(&"package:provider/provider.dart"));
}

#[test]
fn test_extracts_relative_imports() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    let specifiers: Vec<&str> = edges.iter().map(|e| e.specifier.as_str()).collect();
    assert!(specifiers.contains(&"./models/user.dart"));
    assert!(specifiers.contains(&"../utils/helpers.dart"));
}

#[test]
fn test_extracts_export_as_edge() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    let specifiers: Vec<&str> = edges.iter().map(|e| e.specifier.as_str()).collect();
    assert!(
        specifiers.contains(&"src/api.dart"),
        "should contain export 'src/api.dart': {specifiers:?}"
    );
}

#[test]
fn test_names_are_empty_list() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    for edge in &edges {
        assert!(
            edge.names.is_empty(),
            "Dart import '{}' should have empty names",
            edge.specifier
        );
    }
}

#[test]
fn test_edge_count() {
    let edges = extract_imports(DART_SOURCE, "main.dart", "dart");
    assert_eq!(edges.len(), 7, "should have 7 import edges: {edges:?}");
}

#[test]
fn test_no_false_positives_from_non_import_lines() {
    let source = "void main() {\n  print('import ignored');\n}\n";
    let edges = extract_imports(source, "main.dart", "dart");
    assert!(
        edges.is_empty(),
        "non-import lines should produce no edges: {edges:?}"
    );
}

#[test]
fn test_dart_extractor_resolves_for_dart_files() {
    let reg = ExtractorRegistry::default();
    let ext = reg.extractor_for_path(Path::new("main.dart"));
    assert!(ext.is_some(), "Dart extractor must resolve for .dart files");
}
