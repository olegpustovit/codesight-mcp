//! Integration tests: complexity params + nesting depth.

use std::fs;

use codesight_engine::config::AppConfig;
use codesight_engine::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn setup() -> (CodeService, String, tempfile::TempDir) {
    let ws = tempdir().unwrap();
    let src = ws.path().join("src");
    fs::create_dir_all(&src).unwrap();

    fs::write(src.join("p_none.py"), "def foo_none():\n    pass\n").unwrap();
    fs::write(src.join("p_one.py"), "def foo_one(x):\n    pass\n").unwrap();
    fs::write(
        src.join("p_multi.py"),
        "def foo_multi(x, y, z):\n    pass\n",
    )
    .unwrap();
    fs::write(
        src.join("p_generic.py"),
        "def foo_generic(x: dict[str, int], y: int):\n    pass\n",
    )
    .unwrap();

    fs::write(src.join("c_void.c"), "void foo_c_void(void) { }\n").unwrap();
    fs::write(src.join("c_ptr.c"), "void foo_c_ptr(void *ptr) { }\n").unwrap();
    fs::write(
        src.join("c_multi.c"),
        "void foo_c_multi(void *a, int b) { }\n",
    )
    .unwrap();

    fs::write(src.join("n_flat.py"), "def foo_flat():\n    return 1\n").unwrap();
    fs::write(
        src.join("n_one.js"),
        "function foo_one_if() { if (x) { return 1; } }\n",
    )
    .unwrap();
    fs::write(
        src.join("n_deep.js"),
        "function foo_deep() { { { { return 1; } } } }\n",
    )
    .unwrap();

    let svc = codesight_mcp::test_support::make_service_from_config(AppConfig::new(ws.path().join("idx"))).unwrap();
    let r = svc
        .dispatch(
            "ingest_sources",
            json!({ "path": src.to_string_lossy(), "domains": "code" }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "ingest failed: {r:?}");
    let sid = r["data"]["source_id"].as_str().unwrap().to_string();
    (svc, sid, ws)
}

fn find_symbol_id(svc: &CodeService, sid: &str, name: &str) -> String {
    let res = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": name,
                "domain": "code",
                "unit_kinds": ["symbol"],
                "limit": 20,
            }),
        )
        .unwrap();
    assert!(res["ok"].as_bool().unwrap(), "{res:?}");
    for it in res["data"]["items"].as_array().unwrap() {
        if it["ref"]["title"].as_str() == Some(name) {
            return it["ref"]["id"].as_str().unwrap().to_string();
        }
    }
    panic!("symbol {name:?} not found");
}

fn param_count(svc: &CodeService, sid: &str, name: &str) -> u64 {
    let sym = find_symbol_id(svc, sid, name);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["findings"][0]["evidence"]["param_count"]
        .as_u64()
        .expect("missing param_count")
}

fn max_nesting(svc: &CodeService, sid: &str, name: &str) -> u64 {
    let sym = find_symbol_id(svc, sid, name);
    let r = svc
        .dispatch(
            "evaluate_quality",
            json!({
                "source_id": sid,
                "analysis": "complexity",
                "refs": [{ "kind": "symbol", "stable_id": sym }],
            }),
        )
        .unwrap();
    assert!(r["ok"].as_bool().unwrap(), "{r:?}");
    r["data"]["findings"][0]["evidence"]["max_nesting"]
        .as_u64()
        .expect("missing max_nesting")
}

#[test]
fn complexity_params_and_nesting() {
    let (svc, sid, _ws) = setup();

    // param counts
    assert_eq!(param_count(&svc, &sid, "foo_none"), 0, "no params");
    assert_eq!(param_count(&svc, &sid, "foo_one"), 1, "one param");
    assert_eq!(param_count(&svc, &sid, "foo_multi"), 3, "multiple params");
    assert_eq!(param_count(&svc, &sid, "foo_generic"), 2, "nested generic");
    assert_eq!(param_count(&svc, &sid, "foo_c_void"), 0, "c void = zero");
    assert_eq!(param_count(&svc, &sid, "foo_c_ptr"), 1, "c void* = one");
    assert_eq!(param_count(&svc, &sid, "foo_c_multi"), 2, "c void* + int");

    // nesting depth
    assert_eq!(max_nesting(&svc, &sid, "foo_flat"), 0, "flat");
    assert!(max_nesting(&svc, &sid, "foo_one_if") >= 1, "one-level");
    assert!(max_nesting(&svc, &sid, "foo_deep") >= 3, "deeply nested");
}
