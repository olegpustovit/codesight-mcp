//! Integration tests: doc tools.

use std::fs;
use std::path::Path;

use codesight_mcp::config::AppConfig;
use codesight_mcp::sections::should_skip_relative_doc_path;
use codesight_mcp::service::CodeService;
use serde_json::json;
use tempfile::tempdir;

fn doc_fixtures_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_reference")
}

#[test]
fn tools_suite() {
    should_skip_matches_reference_paths();
    doc_index_catalog_toc_search_read_outline_delete();
    doc_section_context_batch_and_meta();
}

fn should_skip_matches_reference_paths() {
    assert!(should_skip_relative_doc_path("build/output.md"));
    assert!(should_skip_relative_doc_path("node_modules/pkg/README.md"));
    assert!(!should_skip_relative_doc_path("rebuild/output.md"));
    assert!(!should_skip_relative_doc_path("docs/guide.md"));
    assert!(should_skip_relative_doc_path("submodule/.git/config"));
    assert!(!should_skip_relative_doc_path("docs/build_notes.md"));
}

fn doc_index_catalog_toc_search_read_outline_delete() {
    let workspace = tempdir().unwrap();
    let fixtures = doc_fixtures_root();
    let doc_root = workspace.path().join("doc-copy");
    fs::create_dir_all(doc_root.join("docs")).unwrap();
    fs::create_dir_all(doc_root.join("text")).unwrap();
    fs::copy(
        fixtures.join("docs/sample.md"),
        doc_root.join("docs/sample.md"),
    )
    .unwrap();
    fs::copy(
        fixtures.join("text/sample.txt"),
        doc_root.join("text/sample.txt"),
    )
    .unwrap();

    let index_root = workspace.path().join("idx");
    let svc = CodeService::new(AppConfig::new(index_root)).unwrap();

    let empty_cat = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(empty_cat["ok"].as_bool().unwrap());
    assert_eq!(
        empty_cat["data"]["sources"].as_array().unwrap().len(),
        0,
        "catalog empty before ingest"
    );

    let bad_path = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": "/nonexistent/path/for/codesight",
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert_eq!(bad_path["ok"], false);

    let not_dir = workspace.path().join("single.md");
    fs::write(&not_dir, "# only file").unwrap();
    let not_dir_r = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": not_dir.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert_eq!(not_dir_r["ok"], false);

    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": doc_root.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    assert!(ing["ok"].as_bool().unwrap());
    assert!(ing["data"]["file_count"].as_u64().unwrap() >= 1);
    assert!(ing["data"]["section_count"].as_u64().unwrap() >= 1);
    let types = ing["data"]["doc_types"].as_array().expect("doc_types");
    assert!(
        types.iter().any(|t| t.as_str() == Some(".md")),
        "expected .md in doc_types: {types:?}"
    );
    assert!(
        types.iter().any(|t| t.as_str() == Some(".txt")),
        "expected .txt in doc_types: {types:?}"
    );
    assert!(ing.get("_meta").is_some());

    let sid = ing["data"]["source_id"].as_str().unwrap();

    let cat = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(cat["ok"].as_bool().unwrap());
    let sources = cat["data"]["sources"].as_array().unwrap();
    assert!(
        sources.iter().any(|s| s["source_id"].as_str() == Some(sid)),
        "catalog lists source"
    );

    let toc = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "toc" }),
        )
        .unwrap();
    assert!(toc["ok"].as_bool().unwrap());
    let toc_items = toc["data"]["items"].as_array().unwrap();
    assert!(!toc_items.is_empty());
    for item in toc_items {
        assert!(
            item.get("content").is_none(),
            "toc entries must not embed full content"
        );
    }
    for i in 1..toc_items.len() {
        let a = &toc_items[i - 1];
        let b = &toc_items[i];
        let pa = (
            a["file"].as_str().unwrap_or(""),
            a["line"].as_u64().unwrap_or(0),
        );
        let pb = (
            b["file"].as_str().unwrap_or(""),
            b["line"].as_u64().unwrap_or(0),
        );
        assert!(pa <= pb, "toc should be sorted by file then line");
    }

    let tree = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "toc_tree" }),
        )
        .unwrap();
    assert!(tree["ok"].as_bool().unwrap());
    let docs = tree["data"]["items"]
        .as_object()
        .expect("toc_tree uses map by file");
    assert!(!docs.is_empty());

    let outline = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "document_outline",
                "path": "docs/sample.md",
            }),
        )
        .unwrap();
    assert!(outline["ok"].as_bool().unwrap());
    assert!(
        outline["data"]["items"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false)
    );

    let missing = svc
        .dispatch(
            "inspect_structure",
            json!({
                "source_id": sid,
                "view": "document_outline",
                "path": "nonexistent.md",
            }),
        )
        .unwrap();
    assert_eq!(missing["ok"], false);

    let search = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "installation",
                "domain": "docs",
                "limit": 10,
            }),
        )
        .unwrap();
    assert!(search["ok"].as_bool().unwrap());
    let hits = search["data"]["items"].as_array().unwrap();
    assert!(!hits.is_empty());
    for h in hits {
        assert!(h.get("content").is_none());
    }
    let capped = svc
        .dispatch(
            "search_catalog",
            json!({
                "source_id": sid,
                "query": "the",
                "domain": "docs",
                "limit": 2,
            }),
        )
        .unwrap();
    assert!(capped["ok"].as_bool().unwrap());
    assert!(capped["data"]["items"].as_array().unwrap().len() <= 2);
    assert!(capped["_meta"].get("tokens_saved").is_some());

    let first_id = toc_items[0]["id"].as_str().unwrap();
    let one = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [{ "kind": "section", "stable_id": first_id }],
            }),
        )
        .unwrap();
    assert!(one["ok"].as_bool().unwrap());
    let c0 = &one["data"]["items"][0]["content"];
    assert!(c0.get("content").is_some());

    let verified = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [{ "kind": "section", "stable_id": first_id }],
                "verify": true,
            }),
        )
        .unwrap();
    assert!(verified["ok"].as_bool().unwrap());
    assert!(
        verified["data"]["items"][0]["content"]
            .get("hash_verified")
            .is_some()
    );

    let bad_sec = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [{ "kind": "section", "stable_id": "nobody::nowhere::nothing#1" }],
            }),
        )
        .unwrap();
    assert!(bad_sec["ok"].as_bool().unwrap());
    assert!(bad_sec["data"]["items"][0].get("error").is_some());

    let ids: Vec<String> = toc_items
        .iter()
        .take(3)
        .filter_map(|x| x["id"].as_str().map(|s| s.to_string()))
        .collect();
    let batch = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": ids
                    .iter()
                    .map(|id| json!({ "kind": "section", "stable_id": id }))
                    .collect::<Vec<_>>(),
            }),
        )
        .unwrap();
    assert!(batch["ok"].as_bool().unwrap());
    assert_eq!(batch["data"]["items"].as_array().unwrap().len(), ids.len());
    let mix = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": [
                    { "kind": "section", "stable_id": &ids[0] },
                    { "kind": "section", "stable_id": "invalid::id::nope#9" },
                ],
            }),
        )
        .unwrap();
    assert!(mix["ok"].as_bool().unwrap());
    assert_eq!(mix["data"]["items"].as_array().unwrap().len(), 2);
    assert!(mix["data"]["items"][0].get("content").is_some());
    assert!(mix["data"]["items"][1].get("error").is_some());

    let meta_batch = svc
        .dispatch(
            "read_content",
            json!({
                "source_id": sid,
                "refs": ids.iter().take(2).map(|id| json!({ "kind": "section", "stable_id": id })).collect::<Vec<_>>(),
            }),
        )
        .unwrap();
    let m = &meta_batch["_meta"];
    assert!(m.get("tokens_saved").is_some());
    assert!(m.get("total_tokens_saved").is_some());
    assert!(m.get("cost_avoided").is_some());
    assert!(m.get("total_cost_avoided").is_some());

    let wipe = svc
        .dispatch(
            "manage_indexes",
            json!({ "action": "delete", "source_id": sid }),
        )
        .unwrap();
    assert!(wipe["ok"].as_bool().unwrap());

    let cat_after = svc.dispatch("catalog_sources", json!({})).unwrap();
    assert!(
        !cat_after["data"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source_id"].as_str() == Some(sid))
    );

    let del_bad = svc
        .dispatch(
            "manage_indexes",
            json!({ "action": "delete", "source_id": "nobody/nothing" }),
        )
        .unwrap();
    assert_eq!(del_bad["ok"], false);
}

fn doc_section_context_batch_and_meta() {
    let workspace = tempdir().unwrap();
    let doc_root = workspace.path().join("p");
    fs::create_dir_all(doc_root.join("docs")).unwrap();
    fs::copy(
        doc_fixtures_root().join("docs/sample.md"),
        doc_root.join("docs/sample.md"),
    )
    .unwrap();
    let svc = CodeService::new(AppConfig::new(workspace.path().join("i"))).unwrap();
    let ing = svc
        .dispatch(
            "ingest_sources",
            json!({
                "path": doc_root.to_string_lossy(),
                "domains": "docs",
                "incremental": true,
            }),
        )
        .unwrap();
    let sid = ing["data"]["source_id"].as_str().unwrap();
    let toc = svc
        .dispatch(
            "inspect_structure",
            json!({ "source_id": sid, "view": "toc" }),
        )
        .unwrap();
    let items = toc["data"]["items"].as_array().unwrap();
    let prereq = items
        .iter()
        .find(|x| x["heading"].as_str() == Some("Prerequisites"))
        .expect("Prerequisites");
    let prereq_id = prereq["id"].as_str().unwrap();

    let ctx = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": prereq_id }],
                "limit": 4,
            }),
        )
        .unwrap();
    assert!(ctx["ok"].as_bool().unwrap());
    let sec = &ctx["data"]["section"];
    assert_eq!(sec["id"], prereq_id);
    let anc = ctx["data"]["ancestors"].as_array().unwrap();
    assert!(!anc.is_empty());
    let titles: Vec<String> = anc
        .iter()
        .filter_map(|a| a["title"].as_str().map(|s| s.to_ascii_lowercase()))
        .collect();
    assert!(
        titles
            .iter()
            .any(|t| t.contains("installation") || t.contains("sample"))
    );

    let adv = items
        .iter()
        .find(|x| x["heading"].as_str() == Some("Advanced Configuration"))
        .expect("Advanced Configuration");
    let actx = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": adv["id"].as_str().unwrap() }],
            }),
        )
        .unwrap();
    let levels: Vec<i64> = actx["data"]["ancestors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["level"].as_i64().unwrap())
        .collect();
    let mut sorted_lv = levels.clone();
    sorted_lv.sort();
    assert_eq!(
        levels, sorted_lv,
        "ancestors root-first ⇒ non-decreasing levels"
    );

    let install = items
        .iter()
        .find(|x| x["heading"].as_str() == Some("Installation"))
        .unwrap();
    let ictx = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": install["id"].as_str().unwrap() }],
            }),
        )
        .unwrap();
    let kids = ictx["data"]["children"].as_array().unwrap();
    assert!(kids.len() >= 2);
    let ctitles: Vec<String> = kids
        .iter()
        .filter_map(|c| c["title"].as_str().map(|s| s.to_ascii_lowercase()))
        .collect();
    assert!(ctitles.iter().any(|t| t.contains("prerequisites")));
    assert!(ctitles.iter().any(|t| t.contains("quick")));

    let no_kids = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": install["id"].as_str().unwrap() }],
                "include_children": false,
            }),
        )
        .unwrap();
    assert_eq!(no_kids["data"]["children"].as_array().unwrap().len(), 0);

    let trunc = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": prereq_id }],
                "max_tokens": 1u32,
            }),
        )
        .unwrap();
    let tsec = &trunc["data"]["section"];
    assert!(
        tsec.get("content_truncated") == Some(&json!(true))
            || tsec["content"].as_str().unwrap().len() <= 8
    );

    let meta = &ctx["_meta"];
    assert!(meta.get("timing_ms").is_some() || meta.get("latency_ms").is_some());
    assert!(meta.get("ancestor_count").is_some());
    assert!(meta.get("child_count").is_some());
    assert!(meta.get("tokens_saved").is_some());

    let bad = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": "nonexistent/repo",
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": "x" }],
            }),
        )
        .unwrap();
    assert_eq!(bad["ok"], false);

    let bad_id = svc
        .dispatch(
            "assemble_context",
            json!({
                "source_id": sid,
                "mode": "section_context",
                "refs": [{ "kind": "section", "stable_id": "nobody::nowhere::nothing#99" }],
            }),
        )
        .unwrap();
    assert_eq!(bad_id["ok"], false);
}
