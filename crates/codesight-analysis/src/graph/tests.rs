use std::collections::{HashMap, HashSet};

use super::coupling::CouplingAssessment;
use super::{LayerDef, compute_coupling, file_to_layer, find_cycles};

// ── find_cycles tests (port of TestFindCycles) ─────────────────────

#[test]
fn empty_graph_returns_no_cycles() {
    assert_eq!(find_cycles(&HashMap::new()), Vec::<Vec<String>>::new());
}

#[test]
fn linear_graph_no_cycle() {
    let adj = HashMap::from([
        ("a".into(), vec!["b".into()]),
        ("b".into(), vec!["c".into()]),
        ("c".into(), vec![]),
    ]);
    assert_eq!(find_cycles(&adj), Vec::<Vec<String>>::new());
}

#[test]
fn self_loop_not_reported() {
    let adj = HashMap::from([("a".into(), vec!["a".into()])]);
    assert_eq!(find_cycles(&adj), Vec::<Vec<String>>::new());
}

#[test]
fn two_node_mutual_cycle() {
    let adj = HashMap::from([
        ("x".into(), vec!["y".into()]),
        ("y".into(), vec!["x".into()]),
    ]);
    let cycles = find_cycles(&adj);
    assert_eq!(cycles.len(), 1);
    assert_eq!(
        cycles[0].iter().collect::<HashSet<_>>(),
        HashSet::from([&"x".to_string(), &"y".to_string()])
    );
}

#[test]
fn three_node_cycle() {
    let adj = HashMap::from([
        ("a".into(), vec!["b".into()]),
        ("b".into(), vec!["c".into()]),
        ("c".into(), vec!["a".into()]),
    ]);
    let cycles = find_cycles(&adj);
    assert_eq!(cycles.len(), 1);
    assert_eq!(
        cycles[0].iter().collect::<HashSet<_>>(),
        HashSet::from([&"a".to_string(), &"b".to_string(), &"c".to_string()])
    );
}

#[test]
fn two_independent_cycles() {
    let adj = HashMap::from([
        ("a".into(), vec!["b".into()]),
        ("b".into(), vec!["a".into()]),
        ("c".into(), vec!["d".into()]),
        ("d".into(), vec!["c".into()]),
        ("e".into(), vec![]),
    ]);
    let cycles = find_cycles(&adj);
    assert_eq!(cycles.len(), 2);
    let sets: Vec<HashSet<&String>> = cycles.iter().map(|c| c.iter().collect()).collect();
    assert!(sets.contains(&HashSet::from([&"a".to_string(), &"b".to_string()])));
    assert!(sets.contains(&HashSet::from([&"c".to_string(), &"d".to_string()])));
}

#[test]
fn isolated_node_not_in_cycle() {
    let adj = HashMap::from([
        ("a".into(), vec!["b".into()]),
        ("b".into(), vec!["a".into()]),
        ("isolated".into(), vec![]),
    ]);
    let cycles = find_cycles(&adj);
    assert_eq!(cycles.len(), 1);
    assert!(!cycles[0].contains(&"isolated".to_string()));
}

#[test]
fn cycle_members_are_sorted() {
    let adj = HashMap::from([
        ("z".into(), vec!["a".into()]),
        ("a".into(), vec!["z".into()]),
    ]);
    let cycles = find_cycles(&adj);
    assert_eq!(cycles[0], {
        let mut v = cycles[0].clone();
        v.sort();
        v
    });
}

// ── file_to_layer tests (port of TestFileToLayer) ──────────────────

fn test_layers() -> Vec<LayerDef> {
    vec![
        LayerDef {
            name: "api".into(),
            paths: vec!["src/api".into()],
            may_not_import: vec![],
        },
        LayerDef {
            name: "svc".into(),
            paths: vec!["src/svc".into()],
            may_not_import: vec![],
        },
        LayerDef {
            name: "db".into(),
            paths: vec!["src/db".into()],
            may_not_import: vec![],
        },
    ]
}

#[test]
fn matches_prefix() {
    assert_eq!(
        file_to_layer("src/api/routes.py", &test_layers()),
        Some("api".into())
    );
}

#[test]
fn matches_db_prefix() {
    assert_eq!(
        file_to_layer("src/db/models.py", &test_layers()),
        Some("db".into())
    );
}

#[test]
fn unassigned_returns_none() {
    assert_eq!(file_to_layer("src/other/utils.py", &test_layers()), None);
}

#[test]
fn exact_path_match() {
    let layers = vec![LayerDef {
        name: "root".into(),
        paths: vec!["src/main.py".into()],
        may_not_import: vec![],
    }];
    assert_eq!(file_to_layer("src/main.py", &layers), Some("root".into()));
}

#[test]
fn no_partial_prefix_match() {
    assert_eq!(file_to_layer("src/apiother/x.py", &test_layers()), None);
}

#[test]
fn empty_layers() {
    assert_eq!(file_to_layer("src/api/x.py", &[]), None);
}

// ── Coupling metrics tests ─────────────────────────────────────────

#[test]
fn coupling_leaf_stable() {
    let adj = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec!["c.py".into()]),
        ("c.py".into(), vec![]),
    ]);
    let m = compute_coupling("c.py", &adj);
    assert_eq!(m.ce, 0);
    assert!(m.ca >= 1);
    assert_eq!(m.instability, Some(0.0));
    assert_eq!(m.assessment, CouplingAssessment::Stable);
}

#[test]
fn coupling_top_unstable() {
    let adj = HashMap::from([
        ("a.py".into(), vec!["b.py".into()]),
        ("b.py".into(), vec![]),
    ]);
    let m = compute_coupling("a.py", &adj);
    assert!(m.ce >= 1);
}
