//! Heuristic dead-code analysis (file and symbol granularity). Used by `evaluate_quality`.

use std::{collections::HashSet, fs, path::Path};

use serde_json::{Value, json};

use crate::storage::IndexStore;

fn glob_match(pattern: &str, path: &str) -> bool {
    let re_str = pattern.replace('.', r"\.").replace('*', "[^/]*");
    regex::Regex::new(&format!("(?:^|/){re_str}$"))
        .map(|re| re.is_match(path))
        .unwrap_or(false)
}

/// Build the `data` object for `evaluate_quality` with `analysis: "dead_code"`.
pub fn dead_code_payload(
    store: &IndexStore,
    granularity: &str,
    min_confidence: f64,
    include_tests: bool,
    entry_patterns: &[String],
) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_for_analysis()?;
    let dep_graph = store.dependency_graph()?;
    let all_files = store.all_file_paths()?;

    let mut referenced_files: HashSet<String> = HashSet::new();
    for targets in dep_graph.values() {
        for t in targets {
            referenced_files.insert(t.clone());
        }
    }

    let is_entry_point = |fp: &str| -> bool {
        let basename = Path::new(fp)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if basename.starts_with("main.") || basename == "__main__.py" {
            return true;
        }
        if basename == "__init__.py" {
            return true;
        }
        for pat in entry_patterns {
            if glob_match(pat, fp) {
                return true;
            }
        }
        false
    };

    let is_test_file = |fp: &str| -> bool {
        let basename = Path::new(fp)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        basename.starts_with("test_")
            || basename.starts_with("tests")
            || fp.contains("/tests/")
            || fp.contains("/test/")
    };

    let root_path = store.root_path().ok();
    let mut files_with_main_guard: HashSet<String> = HashSet::new();
    if let Some(ref root) = root_path {
        for fp in &all_files {
            let abs = root.join(fp);
            if let Ok(content) = fs::read_to_string(&abs)
                && content.contains("__name__")
                && content.contains("__main__")
            {
                files_with_main_guard.insert(fp.clone());
            }
        }
    }
    for sym in &all_symbols {
        if sym.source_text.contains("__name__") && sym.source_text.contains("__main__") {
            files_with_main_guard.insert(sym.file_path.clone());
        }
    }

    let files_with_imports: HashSet<String> = dep_graph
        .iter()
        .filter(|(_, targets)| !targets.is_empty())
        .map(|(src, _)| src.clone())
        .collect();

    let mut dead_files: Vec<Value> = Vec::new();
    let mut live_roots = 0usize;

    for fp in &all_files {
        if !include_tests && is_test_file(fp) {
            continue;
        }
        if is_entry_point(fp) || files_with_main_guard.contains(fp.as_str()) {
            live_roots += 1;
            continue;
        }
        if referenced_files.contains(fp.as_str()) {
            live_roots += 1;
            continue;
        }
        let conf = if files_with_imports.contains(fp.as_str()) {
            0.7
        } else {
            1.0
        };
        if conf >= min_confidence {
            dead_files.push(json!({
                "file": fp,
                "confidence": conf,
            }));
        }
    }

    let mut dead_symbols: Vec<Value> = Vec::new();
    if granularity == "symbol" {
        let dead_file_set: HashSet<&str> = dead_files
            .iter()
            .filter_map(|d| d["file"].as_str())
            .collect();
        for sym in &all_symbols {
            if !include_tests && is_test_file(&sym.file_path) {
                continue;
            }
            if !dead_file_set.contains(sym.file_path.as_str()) {
                continue;
            }
            dead_symbols.push(json!({
                "symbol_id": sym.id,
                "name": sym.name,
                "kind": sym.kind,
                "file": sym.file_path,
                "line": sym.start_line,
                "confidence": 0.8,
            }));
        }
    }

    let dead_file_count = dead_files.len();
    let dead_symbol_count = dead_symbols.len();

    let entry_point_basenames = [
        "main.py",
        "__main__.py",
        "__init__.py",
        "app.py",
        "application.py",
        "wsgi.py",
        "asgi.py",
        "manage.py",
        "setup.py",
        "conftest.py",
        "main.js",
        "index.js",
        "app.js",
        "server.js",
        "main.ts",
        "index.ts",
        "app.ts",
        "server.ts",
        "main.go",
        "main.rs",
        "main.c",
        "main.cpp",
    ];
    let has_entry_points = all_files.iter().any(|fp| {
        let basename = Path::new(fp)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        entry_point_basenames.contains(&basename)
    }) || !files_with_main_guard.is_empty();

    let mut data = json!({
        "analysis": "dead_code",
        "dead_files": dead_files,
        "dead_file_count": dead_file_count,
        "dead_symbols": if granularity == "symbol" { json!(dead_symbols) } else { json!([]) },
        "dead_symbol_count": if granularity == "symbol" { dead_symbol_count } else { 0 },
        "live_root_count": live_roots,
        "analysis_notes": format!("heuristic dead code detection (granularity={granularity})"),
    });

    if !has_entry_points {
        data["framework_warning"] = json!(
            "No standard entry points detected (e.g. main.py, app.py). \
             Dead code results may be inaccurate without known entry points. \
             Consider specifying entry_point_patterns."
        );
    }

    Ok(data)
}

/// Build the `data` object for `evaluate_quality` with `analysis: "dead_code_v2"`.
pub fn dead_code_v2_payload(store: &IndexStore, min_confidence: f64) -> anyhow::Result<Value> {
    let all_symbols = store.all_symbols_for_analysis()?;
    let dep_graph = store.dependency_graph()?;

    let mut referenced_files: HashSet<String> = HashSet::new();
    for targets in dep_graph.values() {
        for t in targets {
            referenced_files.insert(t.clone());
        }
    }

    let mut dead_symbols: Vec<Value> = Vec::new();
    let total_analysed = all_symbols.len();

    for sym in &all_symbols {
        let mut signals: Vec<&str> = Vec::new();
        let mut confidence: f64 = 0.0;

        let basename = Path::new(&sym.file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if basename.starts_with("main.") || basename == "__main__.py" {
            continue;
        }
        if basename.starts_with("test_")
            || sym.file_path.contains("/tests/")
            || sym.file_path.contains("/test/")
        {
            continue;
        }
        if basename == "__init__.py" {
            continue;
        }

        let file_is_unreachable = !referenced_files.contains(sym.file_path.as_str());
        if file_is_unreachable {
            signals.push("unreachable_file");
            confidence += 0.5;
        }

        let ref_count = store.symbol_reference_count(&sym.name, &sym.id)?;
        if ref_count == 0 {
            signals.push("no_callers");
            confidence += 0.3;
        }

        let is_exported = all_symbols.iter().any(|other| {
            other.id != sym.id
                && other.file_path.ends_with("__init__.py")
                && other.source_text.contains(&sym.name)
        });
        if !is_exported && !signals.is_empty() {
            signals.push("not_barrel_exported");
            confidence += 0.2;
        }

        confidence = confidence.min(1.0);

        if !signals.is_empty() && confidence >= min_confidence {
            dead_symbols.push(json!({
                "id": sym.id,
                "name": sym.name,
                "kind": sym.kind,
                "file": sym.file_path,
                "line": sym.start_line,
                "confidence": confidence,
                "signals": signals,
            }));
        }
    }

    Ok(json!({
        "analysis": "dead_code_v2",
        "dead_symbols": dead_symbols,
        "total_analysed": total_analysed,
    }))
}
