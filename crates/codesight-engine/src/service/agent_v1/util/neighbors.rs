//! Outline-neighbor walk and heading-path ancestor helpers for
//! section-oriented responses (context_bundle, inspect_structure, etc.).

use serde_json::{Value, json};

fn append_section_neighbor(
    out: &mut Vec<Value>,
    row: &Value,
    source_id: &str,
    skip_id: &str,
) -> bool {
    let Some(nid) = row.get("id").and_then(|x| x.as_str()) else {
        return false;
    };
    if nid.is_empty() || nid == skip_id {
        return false;
    }
    out.push(json!({
        "ref": {
            "kind": "section",
            "stable_id": nid,
            "source_id": source_id,
        },
        "heading": row.get("heading"),
        "line": row.get("line"),
        "summary": row.get("summary"),
    }));
    true
}

fn heading_path_ancestor_prefixes(heading_path: &str) -> Vec<String> {
    let parts: Vec<&str> = heading_path.split(" > ").collect();
    if parts.len() <= 1 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..parts.len() - 1 {
        out.push(parts[..=i].join(" > "));
    }
    out
}

pub(in crate::service) fn heading_path_ancestors(all: &[Value], heading_path: &str) -> Vec<Value> {
    let mut acc = Vec::new();
    for pref in heading_path_ancestor_prefixes(heading_path) {
        if let Some(row) = all
            .iter()
            .find(|r| r.get("heading_path").and_then(|x| x.as_str()) == Some(pref.as_str()))
        {
            acc.push(json!({
                "title": row.get("heading"),
                "id": row.get("id"),
                "level": row.get("level"),
                "line": row.get("line"),
            }));
        }
    }
    acc
}

pub(in crate::service) fn direct_child_sections(
    all: &[Value],
    parent_heading_path: &str,
) -> Vec<Value> {
    let prefix = format!("{} > ", parent_heading_path);
    all.iter()
        .filter_map(|row| {
            let hp = row.get("heading_path").and_then(|x| x.as_str())?;
            if !hp.starts_with(&prefix) {
                return None;
            }
            let rest = &hp[prefix.len()..];
            if rest.contains(" > ") {
                return None;
            }
            Some(json!({
                "title": row.get("heading"),
                "id": row.get("id"),
                "level": row.get("level"),
                "line": row.get("line"),
                "summary": row.get("summary"),
            }))
        })
        .collect()
}

pub(in crate::service) fn outline_neighbors(
    all: &[Value],
    sec_id: &str,
    max_neighbors: usize,
    source_id: &str,
) -> Vec<Value> {
    let idx = all
        .iter()
        .position(|row| row.get("id").and_then(|x| x.as_str()) == Some(sec_id));
    let mut out: Vec<Value> = Vec::new();
    let Some(idx) = idx else {
        return out;
    };
    let mut offset = 1usize;
    while out.len() < max_neighbors {
        let mut progressed = false;
        if idx >= offset
            && let Some(row) = all.get(idx - offset)
            && append_section_neighbor(&mut out, row, source_id, sec_id)
        {
            progressed = true;
        }
        if out.len() >= max_neighbors {
            break;
        }
        if idx + offset < all.len()
            && let Some(row) = all.get(idx + offset)
            && append_section_neighbor(&mut out, row, source_id, sec_id)
        {
            progressed = true;
        }
        if !progressed {
            break;
        }
        offset += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section_row(id: &str, heading: &str, heading_path: &str, line: u32, level: u32) -> Value {
        json!({
            "id": id,
            "heading": heading,
            "heading_path": heading_path,
            "line": line,
            "level": level,
            "summary": format!("summary of {heading}"),
        })
    }

    #[test]
    fn heading_path_ancestors_returns_chain_excluding_self() {
        let all = vec![
            section_row("r", "Root", "Root", 1, 1),
            section_row("c", "Child", "Root > Child", 2, 2),
            section_row("g", "Grand", "Root > Child > Grand", 3, 3),
        ];
        let anc = heading_path_ancestors(&all, "Root > Child > Grand");
        assert_eq!(anc.len(), 2);
        assert_eq!(anc[0]["id"], "r");
        assert_eq!(anc[1]["id"], "c");
    }

    #[test]
    fn heading_path_ancestors_empty_for_top_level() {
        let all = vec![section_row("r", "Root", "Root", 1, 1)];
        assert!(heading_path_ancestors(&all, "Root").is_empty());
    }

    #[test]
    fn direct_child_sections_only_returns_depth_one_descendants() {
        let all = vec![
            section_row("r", "Root", "Root", 1, 1),
            section_row("c1", "Child1", "Root > Child1", 2, 2),
            section_row("c2", "Child2", "Root > Child2", 4, 2),
            section_row("g", "Grand", "Root > Child1 > Grand", 3, 3),
        ];
        let kids = direct_child_sections(&all, "Root");
        assert_eq!(kids.len(), 2);
        let ids: Vec<&str> = kids.iter().map(|k| k["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"c1"));
        assert!(ids.contains(&"c2"));
    }

    #[test]
    fn outline_neighbors_walks_symmetrically_and_skips_self() {
        let all = vec![
            section_row("a", "A", "A", 1, 1),
            section_row("b", "B", "A > B", 2, 2),
            section_row("c", "C", "A > C", 3, 2),
            section_row("d", "D", "A > D", 4, 2),
            section_row("e", "E", "A > E", 5, 2),
        ];
        let out = outline_neighbors(&all, "c", 4, "src");
        assert_eq!(out.len(), 4);
        assert_eq!(out[0]["ref"]["stable_id"], "b");
        assert_eq!(out[1]["ref"]["stable_id"], "d");
        assert_eq!(out[2]["ref"]["stable_id"], "a");
        assert_eq!(out[3]["ref"]["stable_id"], "e");
        for item in &out {
            assert_eq!(item["ref"]["kind"], "section");
            assert_eq!(item["ref"]["source_id"], "src");
        }
    }

    #[test]
    fn outline_neighbors_returns_empty_when_anchor_missing() {
        let all = vec![section_row("a", "A", "A", 1, 1)];
        assert!(outline_neighbors(&all, "missing", 5, "src").is_empty());
    }

    #[test]
    fn outline_neighbors_stops_at_max() {
        let all: Vec<Value> = (0..10)
            .map(|i| section_row(&format!("s{i}"), "S", "S", i, 1))
            .collect();
        let out = outline_neighbors(&all, "s5", 2, "src");
        assert_eq!(out.len(), 2);
    }
}
