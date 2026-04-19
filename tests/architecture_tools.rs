//! Integration tests: architecture tools.

use std::fs;
use std::path::Path;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Fixture builders ───────────────────────────────────────────────────────

/// Three files with no cycles: a → b → c.
fn build_linear_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("c.py"), "def leaf():\n    pass\n").unwrap();
    fs::write(
        src.join("b.py"),
        "from c import leaf\n\ndef mid():\n    return leaf()\n",
    )
    .unwrap();
    fs::write(
        src.join("a.py"),
        "from b import mid\n\ndef top():\n    return mid()\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

/// Two files with a mutual import cycle: x ↔ y, plus standalone z.
fn build_cyclic_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(
        src.join("x.py"),
        "from y import bar\n\ndef foo():\n    return bar()\n",
    )
    .unwrap();
    fs::write(
        src.join("y.py"),
        "from x import foo\n\ndef bar():\n    return foo()\n",
    )
    .unwrap();
    fs::write(src.join("z.py"), "def standalone():\n    pass\n").unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

/// Flat repo with layer-violating imports:
///   routes.py → models.py (VIOLATION: api may_not_import db)
///   routes.py → service.py (OK)
///   service.py → models.py (OK)
fn build_layered_repo() -> (tempfile::TempDir, CodeService, String) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("models.py"), "class User:\n    pass\n").unwrap();
    fs::write(
        src.join("service.py"),
        "from models import User\n\ndef get_user():\n    return User()\n",
    )
    .unwrap();
    fs::write(
        src.join("routes.py"),
        "from models import User\nfrom service import get_user\n\ndef handle():\n    return get_user()\n",
    )
    .unwrap();

    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let sid = index_code(&svc, &src);
    (ws, svc, sid)
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn index_code(svc: &CodeService, root: &Path) -> String {
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    r["data"]["source_id"].as_str().unwrap().to_string()
}

fn get_cycles(svc: &CodeService, sid: &str) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "dependency_cycles",
        }),
    )
    .unwrap()
}

fn get_coupling(svc: &CodeService, sid: &str, module_path: &str) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "module_coupling",
            "module_path": module_path,
        }),
    )
    .unwrap()
}

fn get_layer_violations(svc: &CodeService, sid: &str, rules: Value) -> Value {
    svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": sid,
            "graph": "layer_violations",
            "rules": rules,
        }),
    )
    .unwrap()
}

fn find_file_path(svc: &CodeService, sid: &str, suffix: &str) -> String {
    let r = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "file_tree" }),
        )
        .unwrap();
    let data = &r["data"];
    // Walk the tree to find a file ending with suffix
    fn find_in_tree(node: &Value, suffix: &str) -> Option<String> {
        if let Some(path) = node["path"].as_str()
            && path.ends_with(suffix)
        {
            return Some(path.to_string());
        }
        if let Some(children) = node["children"].as_array() {
            for child in children {
                if let Some(found) = find_in_tree(child, suffix) {
                    return Some(found);
                }
            }
        }
        None
    }
    if let Some(items) = data["items"].as_array() {
        for item in items {
            if let Some(found) = find_in_tree(item, suffix) {
                return found;
            }
        }
    }
    panic!("file ending with '{suffix}' not found in file tree: {data:?}");
}

fn layer_rules() -> Value {
    json!([
        {"name": "api", "paths": ["routes.py"], "may_not_import": ["db"]},
        {"name": "svc", "paths": ["service.py"], "may_not_import": []},
        {"name": "db",  "paths": ["models.py"], "may_not_import": []},
    ])
}

// ── dependency_cycles integration tests ────────────────────────────────────

#[test]
fn arch_no_cycles_in_linear_repo() {
    let (_ws, svc, sid) = build_linear_repo();
    let r = get_cycles(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap(), "expected ok: {r:?}");
    assert_eq!(r["data"]["cycle_count"].as_u64().unwrap(), 0);
    assert_eq!(r["data"]["cycles"].as_array().unwrap().len(), 0);
}

#[test]
fn arch_cycle_detected_in_cyclic_repo() {
    let (_ws, svc, sid) = build_cyclic_repo();
    let r = get_cycles(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap(), "expected ok: {r:?}");
    assert!(r["data"]["cycle_count"].as_u64().unwrap() >= 1);
    let cycles = r["data"]["cycles"].as_array().unwrap();
    let found = cycles.iter().any(|cycle| {
        let members: Vec<&str> = cycle
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        members.iter().any(|f| f.ends_with("x.py")) && members.iter().any(|f| f.ends_with("y.py"))
    });
    assert!(found, "Expected x/y cycle, got: {cycles:?}");
}

#[test]
fn arch_standalone_file_not_in_cycle() {
    let (_ws, svc, sid) = build_cyclic_repo();
    let r = get_cycles(&svc, &sid);
    let cycles = r["data"]["cycles"].as_array().unwrap();
    for cycle in cycles {
        let members: Vec<&str> = cycle
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            !members.iter().any(|f| f.ends_with("z.py")),
            "standalone z.py should not be in a cycle"
        );
    }
}

#[test]
fn arch_cycles_result_shape() {
    let (_ws, svc, sid) = build_linear_repo();
    let r = get_cycles(&svc, &sid);
    assert!(r["ok"].as_bool().unwrap());
    let data = &r["data"];
    assert!(data["repo"].is_string());
    assert!(data["cycle_count"].is_number());
    assert!(data["cycles"].is_array());
    assert!(r["_meta"].is_object());
}

#[test]
fn arch_unknown_repo_cycles_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "no_such_repo",
                "graph": "dependency_cycles",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn arch_cycle_count_matches_cycles_list() {
    let (_ws, svc, sid) = build_cyclic_repo();
    let r = get_cycles(&svc, &sid);
    let data = &r["data"];
    assert_eq!(
        data["cycle_count"].as_u64().unwrap(),
        data["cycles"].as_array().unwrap().len() as u64
    );
}

// ── module_coupling integration tests ─────────────────────────────────────

#[test]
fn arch_coupling_result_shape() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "b.py");
    let r = get_coupling(&svc, &sid, &path);
    assert!(r["ok"].as_bool().unwrap(), "expected ok: {r:?}");
    let data = &r["data"];
    for key in [
        "repo",
        "module",
        "ca",
        "ce",
        "instability",
        "assessment",
        "importers",
        "dependencies",
    ] {
        assert!(!data[key].is_null(), "missing key '{key}': {data:?}");
    }
    assert!(r["_meta"].is_object());
}

#[test]
fn arch_coupling_leaf_node_stable() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "c.py");
    let r = get_coupling(&svc, &sid, &path);
    let data = &r["data"];
    assert_eq!(data["ce"].as_u64().unwrap(), 0);
    assert!(data["ca"].as_u64().unwrap() >= 1);
    assert_eq!(data["instability"].as_f64().unwrap(), 0.0);
    assert_eq!(data["assessment"].as_str().unwrap(), "stable");
}

#[test]
fn arch_coupling_top_node_ce_positive() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "a.py");
    let r = get_coupling(&svc, &sid, &path);
    assert!(r["data"]["ce"].as_u64().unwrap() >= 1);
}

#[test]
fn arch_coupling_importers_list_correct() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "b.py");
    let r = get_coupling(&svc, &sid, &path);
    let importers: Vec<&str> = r["data"]["importers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        importers.iter().any(|i| i.ends_with("a.py")),
        "b.py importers should contain a.py: {importers:?}"
    );
}

#[test]
fn arch_coupling_dependencies_list_correct() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "b.py");
    let r = get_coupling(&svc, &sid, &path);
    let deps: Vec<&str> = r["data"]["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        deps.iter().any(|d| d.ends_with("c.py")),
        "b.py dependencies should contain c.py: {deps:?}"
    );
}

#[test]
fn arch_coupling_instability_range() {
    let (_ws, svc, sid) = build_linear_repo();
    let path = find_file_path(&svc, &sid, "b.py");
    let r = get_coupling(&svc, &sid, &path);
    if let Some(i) = r["data"]["instability"].as_f64() {
        assert!(
            (0.0..=1.0).contains(&i),
            "instability should be 0..1, got {i}"
        );
    }
}

#[test]
fn arch_coupling_unknown_file_returns_error() {
    let (_ws, svc, sid) = build_linear_repo();
    let r = get_coupling(&svc, &sid, "no/such/file.py");
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn arch_coupling_unknown_repo_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "ghost/repo",
                "graph": "module_coupling",
                "module_path": "x.py",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

// ── layer_violations integration tests ─────────────────────────────────────

#[test]
fn arch_layer_violations_result_shape() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, layer_rules());
    assert!(r["ok"].as_bool().unwrap(), "expected ok: {r:?}");
    let data = &r["data"];
    for key in ["repo", "layer_count", "violation_count", "violations"] {
        assert!(!data[key].is_null(), "missing key '{key}': {data:?}");
    }
    assert!(r["_meta"].is_object());
}

#[test]
fn arch_layer_violation_detected() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, layer_rules());
    let data = &r["data"];
    assert!(
        data["violation_count"].as_u64().unwrap() >= 1,
        "expected at least 1 violation: {data:?}"
    );
    let api_to_db: Vec<&Value> = data["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| {
            v["file_layer"].as_str() == Some("api") && v["target_layer"].as_str() == Some("db")
        })
        .collect();
    assert!(
        !api_to_db.is_empty(),
        "Expected api→db violation, got: {:?}",
        data["violations"]
    );
}

#[test]
fn arch_layer_violation_fields() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, layer_rules());
    for v in r["data"]["violations"].as_array().unwrap() {
        assert!(v["file"].is_string(), "missing 'file': {v:?}");
        assert!(v["file_layer"].is_string(), "missing 'file_layer': {v:?}");
        assert!(
            v["import_target"].is_string(),
            "missing 'import_target': {v:?}"
        );
        assert!(
            v["target_layer"].is_string(),
            "missing 'target_layer': {v:?}"
        );
        assert!(
            v["rule_violated"].is_string(),
            "missing 'rule_violated': {v:?}"
        );
    }
}

#[test]
fn arch_no_false_positives_for_allowed_imports() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, layer_rules());
    for v in r["data"]["violations"].as_array().unwrap() {
        assert_eq!(
            v["file_layer"].as_str().unwrap(),
            "api",
            "Unexpected violating layer: {v:?}"
        );
    }
}

#[test]
fn arch_no_rules_returns_note() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, json!([]));
    assert!(r["ok"].as_bool().unwrap());
    assert_eq!(r["data"]["violation_count"].as_u64().unwrap(), 0);
    assert!(r["data"]["note"].is_string(), "expected note field: {r:?}");
}

#[test]
fn arch_layer_count_matches_rules() {
    let (_ws, svc, sid) = build_layered_repo();
    let rules = layer_rules();
    let r = get_layer_violations(&svc, &sid, rules.clone());
    assert_eq!(
        r["data"]["layer_count"].as_u64().unwrap(),
        rules.as_array().unwrap().len() as u64
    );
}

#[test]
fn arch_unknown_repo_layer_violations_returns_error() {
    let ws = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "inspect_graph",
            json!({
                "source_id": "ghost/repo",
                "graph": "layer_violations",
            }),
        )
        .unwrap();
    assert!(!r["ok"].as_bool().unwrap_or(true), "expected error: {r:?}");
}

#[test]
fn arch_rule_violated_format() {
    let (_ws, svc, sid) = build_layered_repo();
    let r = get_layer_violations(&svc, &sid, layer_rules());
    for v in r["data"]["violations"].as_array().unwrap() {
        let rule = v["rule_violated"].as_str().unwrap();
        assert!(
            rule.contains("may_not_import"),
            "rule_violated should contain 'may_not_import': {rule}"
        );
    }
}
