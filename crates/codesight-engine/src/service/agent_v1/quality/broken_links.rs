//! `evaluate_quality analysis=broken_links` — markdown internal link check.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

static MD_INTERNAL_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").expect("infallible markdown link regex")
});

impl CodeService {
    pub(super) fn quality_broken_links(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let doc_store =
            require_store!(doc: self, &args.source_id, args.repo_path.as_deref(), timer);
        let all_sections = doc_store.all_sections()?;
        let all_files: HashSet<String> = doc_store.all_doc_files()?.into_iter().collect();

        let mut file_headings: HashMap<String, HashSet<String>> = HashMap::new();
        for sec in &all_sections {
            file_headings
                .entry(sec.file_path.clone())
                .or_default()
                .insert(sec.heading.to_lowercase().replace(' ', "-"));
        }

        let mut broken_links: Vec<Value> = Vec::new();
        for sec in &all_sections {
            for cap in MD_INTERNAL_LINK_RE.captures_iter(&sec.content_text) {
                let Some(target) = cap.get(2).map(|m| m.as_str()) else {
                    continue;
                };
                if let Some(entry) = check_internal_link(sec, target, &all_files, &file_headings) {
                    broken_links.push(entry);
                }
            }
        }

        self.success_response(
            timer,
            json!({
                "analysis": "broken_links",
                "repo": args.source_id,
                "docs_scanned": all_files.len(),
                "sections_scanned": all_sections.len(),
                "broken_link_count": broken_links.len(),
                "broken_links": broken_links,
            }),
        )
    }
}

fn check_internal_link(
    sec: &codesight_storage::storage::doc_index::SectionRecord,
    target: &str,
    all_files: &HashSet<String>,
    file_headings: &HashMap<String, HashSet<String>>,
) -> Option<Value> {
    if target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
    {
        return None;
    }
    let (file_part, anchor_part) = match target.find('#') {
        Some(pos) => (&target[..pos], Some(&target[pos + 1..])),
        None => (target, None),
    };

    let resolved_file = if file_part.is_empty() {
        sec.file_path.clone()
    } else {
        let base_dir = std::path::Path::new(&sec.file_path)
            .parent()
            .unwrap_or(std::path::Path::new(""));
        base_dir
            .join(file_part)
            .to_string_lossy()
            .replace('\\', "/")
    };

    let reason_detail = if !file_part.is_empty() && !all_files.contains(&resolved_file) {
        Some(("file_not_found", format!("file not found: {resolved_file}")))
    } else if let Some(anchor) = anchor_part {
        let anchor_lower = anchor.to_lowercase();
        let headings_key = if file_part.is_empty() {
            &sec.file_path
        } else {
            &resolved_file
        };
        match file_headings.get(headings_key) {
            Some(headings) if !headings.contains(&anchor_lower) => Some((
                "section_not_found",
                if file_part.is_empty() {
                    format!("anchor '{anchor}' not found")
                } else {
                    format!("anchor '{anchor}' not found in {resolved_file}")
                },
            )),
            _ => None,
        }
    } else {
        None
    };

    reason_detail.map(|(reason_code, detail)| {
        json!({
            "source_file": sec.file_path,
            "source_section": sec.heading,
            "target": target,
            "line": sec.start_line,
            "reason": reason_code,
            "detail": detail,
        })
    })
}
