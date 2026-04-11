//! Integration tests: class hierarchy.

use std::fs;

use codesight_mcp::config::AppConfig;
use codesight_mcp::service::CodeService;
use codesight_mcp::storage::class_hierarchy::parse_bases;
use codesight_mcp::test_support;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn class_hierarchy_suite() {
    case_parse_bases_unit();
    case_errors_and_integration();
    case_external_base();
}

fn case_parse_bases_unit() {
    assert_eq!(parse_bases("class Dog(Animal)"), vec!["Animal"]);
    let m = parse_bases("class Mixin(Base1, Base2)");
    assert!(m.iter().any(|s| s == "Base1"));
    assert!(m.iter().any(|s| s == "Base2"));
    assert!(parse_bases("class Standalone").is_empty());
    assert!(parse_bases("class Root()").is_empty());

    let j = parse_bases("class Dog extends Animal");
    assert!(j.contains(&"Animal".to_string()));
    let j2 = parse_bases("class Dog implements IAnimal, IMovable");
    assert!(j2.contains(&"IAnimal".to_string()));
    assert!(j2.contains(&"IMovable".to_string()));
    let j3 = parse_bases("class Dog extends Animal implements IMovable");
    assert!(j3.contains(&"Animal".to_string()));
    assert!(j3.contains(&"IMovable".to_string()));

    let f = parse_bases("class Foo(lowercase, Bar)");
    assert!(!f.iter().any(|s| s == "lowercase"));
    assert!(f.contains(&"Bar".to_string()));

    assert!(parse_bases("").is_empty());
}

fn ingest(svc: &CodeService, root: &std::path::Path) -> String {
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": root.to_string_lossy(),
                "domains": "code",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap(), "{ing:?}");
    ing["data"]["source_id"].as_str().unwrap().to_string()
}

fn hierarchy_json(svc: &CodeService, source_id: &str, class_name: &str) -> serde_json::Value {
    svc.dispatch(
        "inspect_structure",
        json!({
            "source_id": source_id,
            "view": "class_hierarchy",
            "path": class_name,
        }),
    )
    .unwrap()
}

fn case_errors_and_integration() {
    let workspace = tempdir().unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let res = hierarchy_json(&svc, "no_such_repo__", "Foo");
    assert!(!res["ok"].as_bool().unwrap());

    let root = workspace.path().join("r");
    let src = root.join("src");
    test_support::write_hierarchy_repo(&src).unwrap();
    let sid = ingest(&svc, &root);

    let res = hierarchy_json(&svc, &sid, "NonExistentClass");
    assert!(!res["ok"].as_bool().unwrap());
    assert!(
        res["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("NonExistentClass")
    );

    let animal = hierarchy_json(&svc, &sid, "Animal");
    assert!(animal["ok"].as_bool().unwrap());
    assert_eq!(animal["data"]["ancestor_count"], 0);
    assert_eq!(animal["data"]["ancestors"].as_array().unwrap().len(), 0);

    let mammal = hierarchy_json(&svc, &sid, "Mammal");
    let an: Vec<_> = mammal["data"]["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(an.contains(&"Animal"));

    let dog = hierarchy_json(&svc, &sid, "Dog");
    let da: Vec<_> = dog["data"]["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(da.contains(&"Mammal") && da.contains(&"Animal"));
    let pos_m = da.iter().position(|&x| x == "Mammal").unwrap();
    let pos_a = da.iter().position(|&x| x == "Animal").unwrap();
    assert!(pos_m < pos_a);

    let an_desc: Vec<_> = animal["data"]["descendants"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(an_desc.contains(&"Mammal"));
    assert!(an_desc.contains(&"Dog"));
    assert!(an_desc.contains(&"Cat"));

    let mamm_desc: Vec<_> = mammal["data"]["descendants"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["name"].as_str())
        .collect();
    assert!(mamm_desc.contains(&"Dog") && mamm_desc.contains(&"Cat"));

    let dog_leaf = hierarchy_json(&svc, &sid, "Dog");
    assert_eq!(dog_leaf["data"]["descendant_count"], 0);
    assert!(
        dog_leaf["data"]["descendants"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let low = hierarchy_json(&svc, &sid, "animal");
    assert_eq!(low["data"]["class"]["name"], "Animal");

    assert!(animal.get("_meta").is_some());
    assert!(animal["_meta"].get("timing_ms").is_some());

    let info = &animal["data"]["class"];
    assert_eq!(info["name"], "Animal");
    assert!(info.get("file").is_some());
    assert!(info.get("line").is_some());
}

fn case_external_base() {
    let workspace = tempdir().unwrap();
    let root = workspace.path().join("r");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("derived.py"),
        "class MyWidget(ExternalBase):\n    pass\n",
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("idx"))).unwrap();
    let sid = ingest(&svc, &root);
    let res = hierarchy_json(&svc, &sid, "MyWidget");
    assert!(res["ok"].as_bool().unwrap());
    let anc = res["data"]["ancestors"].as_array().unwrap();
    let ext: Vec<_> = anc
        .iter()
        .filter(|x| x["name"].as_str() == Some("ExternalBase"))
        .collect();
    assert!(!ext.is_empty());
    assert_eq!(ext[0]["file"].as_str(), Some("(external)"));
}
