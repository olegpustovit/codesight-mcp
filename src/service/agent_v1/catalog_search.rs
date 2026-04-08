//! Search catalog (BM25, semantic, docs).

use std::collections::HashSet;

use crate::fuzzy;
use crate::model::SearchSymbol;
use crate::savings::MetaBuilder;
use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::*;
use crate::service::args::parse_args;
use crate::service::helpers::{ToolError, index_error};
use crate::service::{CodeService, MAX_QUERY_LEN};
use serde_json::{Value, json};

impl CodeService {
    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let args: SearchCatalogArgs = parse_args(v)?;

        // Validate sort_by early
        if let Some(ref sort_by) = args.sort_by {
            let valid = [
                "relevance",
                "centrality",
                "name",
                "file",
                "line",
                "combined",
            ];
            if !valid.contains(&sort_by.as_str()) {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "INVALID_REQUEST",
                        message: format!(
                            "Invalid sort_by value '{}'. Valid values: {:?}",
                            sort_by, valid
                        ),
                        detail: json!({ "sort_by": sort_by }),
                    },
                );
            }
        }

        // Handle semantic search
        let semantic_enabled = args.semantic.unwrap_or(false);
        let semantic_only = args.semantic_only.unwrap_or(false);

        if semantic_enabled || semantic_only {
            let provider = crate::semantic::detect_embedding_provider();
            if provider.is_none() {
                return self.error_response(
                    &MetaBuilder::new(0),
                    ToolError {
                        code: "no_embedding_provider",
                        message: "No embedding provider configured. Set CODESIGHT_EMBED_MODEL, or OPENAI_API_KEY+OPENAI_EMBED_MODEL, or GOOGLE_API_KEY+GOOGLE_EMBED_MODEL.".into(),
                        detail: json!({}),
                    },
                );
            }
        }

        let scope = args.scope.clone().or(args.file_pattern.clone());
        let lim = args.limit.clamp(1, 50);
        let domain = args
            .domain
            .unwrap_or_else(|| "hybrid".to_string())
            .to_ascii_lowercase();
        let kinds = args.unit_kinds.clone();

        let mut items: Vec<Value> = Vec::new();
        let mut warnings = Vec::new();

        let want_symbol = kinds
            .as_ref()
            .map(|k| k.iter().any(|x| x == "symbol"))
            .unwrap_or(true)
            && domain != "docs";
        let want_section = kinds
            .as_ref()
            .map(|k| k.iter().any(|x| x == "section"))
            .unwrap_or(false)
            || domain == "docs"
            || domain == "hybrid";
        let want_file = kinds
            .as_ref()
            .map(|k| k.iter().any(|x| x == "file"))
            .unwrap_or(false);

        let need_code_index = domain != "docs" && (want_symbol || want_file);
        let need_doc_index =
            domain != "code" && (want_section || domain == "docs" || domain == "hybrid");

        let code_store = if need_code_index {
            match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" => {
                    warnings.push(json!({
                        "code": "CODE_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            }
        } else {
            None
        };

        let doc_store = if need_doc_index {
            match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" && code_store.is_some() => {
                    warnings.push(json!({
                        "code": "DOC_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) if domain == "docs" || domain == "hybrid" => {
                    return self.error_response(&MetaBuilder::new(0), e);
                }
                Err(_) => None,
            }
        } else {
            None
        };

        if code_store.is_none() && doc_store.is_none() {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    code: "NOT_INDEXED",
                    message: format!(
                        "Source '{}' has no usable index for this search.",
                        args.source_id
                    ),
                    detail: json!({ "source_id": args.source_id, "domain": domain }),
                },
            );
        }

        let per_kind_cap = if domain == "hybrid" {
            (lim / 2).max(1)
        } else {
            lim
        };

        let mut raw_bytes = 0u64;
        let mut more_file_hits_exist = false;

        if let Some(ref store) = code_store {
            raw_bytes = raw_bytes.max(store.repo_total_bytes()?);
            if want_file {
                let qmode = args.query_mode.as_deref().unwrap_or("keyword");
                let ctx = args.context_lines.unwrap_or(0).min(50);
                // Request one extra hit so we can set `truncated` when the cap cuts off further matches.
                let probe_cap = per_kind_cap.saturating_add(1).clamp(1, 50);
                let matches = self.search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.clone(),
                    probe_cap,
                    qmode,
                    ctx,
                )?;
                more_file_hits_exist |= matches.len() > per_kind_cap;
                let mut matches = matches;
                matches.truncate(per_kind_cap);
                items.extend(matches);
            }
            if want_symbol || (!want_file && !want_section) {
                let mut symbols = match self.run_search(
                    store,
                    args.query.clone(),
                    scope.clone(),
                    args.kind.clone(),
                    per_kind_cap,
                ) {
                    Ok(s) => s,
                    Err(e) => return self.error_response(&MetaBuilder::new(raw_bytes.max(1)), e),
                };
                let fuzzy_explicit = args.fuzzy == Some(true);
                let auto_fuzzy = args.fuzzy != Some(true)
                    && symbols.is_empty()
                    && args.fuzzy_threshold.is_some();
                let run_fuzzy = fuzzy_explicit || auto_fuzzy;
                if fuzzy_explicit && !symbols.is_empty() {
                    for s in &mut symbols {
                        s.match_type = Some("exact".to_string());
                    }
                }
                if run_fuzzy {
                    let threshold = args.fuzzy_threshold.unwrap_or(0.3);
                    let primary_ids: HashSet<String> =
                        symbols.iter().map(|s| s.id.clone()).collect();
                    let scan_cap = per_kind_cap.saturating_mul(50).clamp(500, 12_000);
                    if let Ok(candidates) =
                        store.symbols_light_scan(args.kind.as_deref(), scope.as_deref(), scan_cap)
                    {
                        let mut fuzzy_rows: Vec<SearchSymbol> = Vec::new();
                        for mut c in candidates {
                            if primary_ids.contains(&c.id) {
                                continue;
                            }
                            let score = fuzzy::score_name_pair(&args.query, &c.name);
                            if fuzzy::fuzzy_candidate_passes(
                                &score,
                                threshold,
                                args.max_edit_distance,
                            ) {
                                if !auto_fuzzy {
                                    c.match_type = Some("fuzzy".to_string());
                                    c.fuzzy_similarity = Some(score.similarity);
                                    c.edit_distance = Some(score.edit_distance);
                                }
                                c.score = score.similarity * 0.45;
                                fuzzy_rows.push(c);
                            }
                        }
                        fuzzy_rows.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                                .then_with(|| a.id.cmp(&b.id))
                        });
                        let take_extra = per_kind_cap.saturating_mul(4).clamp(32, 100);
                        fuzzy_rows.truncate(take_extra);
                        symbols.extend(fuzzy_rows);
                    }
                }
                for s in symbols {
                    items.push(search_hit_symbol(&args.source_id, &s));
                }
            }
        }

        if let Some(ref dstore) = doc_store {
            raw_bytes = raw_bytes.max(dstore.repo_total_bytes()?);
            if want_section || domain == "docs" || domain == "hybrid" {
                let trimmed = args.query.trim();
                if !trimmed.is_empty() {
                    let effective_query = if trimmed.len() > MAX_QUERY_LEN {
                        &trimmed[..MAX_QUERY_LEN]
                    } else {
                        trimmed
                    };
                    let sections = match dstore.search_sections(
                        effective_query,
                        scope.as_deref(),
                        per_kind_cap,
                    ) {
                        Ok(s) => s,
                        Err(e) => {
                            return self.error_response(
                                &MetaBuilder::new(raw_bytes.max(1)),
                                index_error(e),
                            );
                        }
                    };
                    for sec in sections {
                        items.push(json!({
                            "ref": {
                                "id": sec.id,
                                "stable_id": sec.id,
                                "source_id": args.source_id,
                                "domain": "docs",
                                "kind": "section",
                                "path": sec.file,
                                "title": sec.heading,
                                "summary": sec.summary,
                            },
                            "snippet": sec.heading_path,
                            "score": sec.score,
                            "line": sec.line,
                        }));
                    }
                }
            }
        }

        // ── Semantic search integration ────────────────────────────────────
        let search_mode = if semantic_only {
            "semantic_only"
        } else if semantic_enabled {
            "hybrid"
        } else {
            "bm25"
        };

        if (semantic_enabled || semantic_only) && want_symbol {
            let emb_path = self.config.embedding_path(&args.source_id);
            let model_path = emb_path.with_extension("tfidf.json");
            if emb_path.exists() && model_path.exists() {
                let emb_store = crate::semantic::EmbeddingStore::open(&emb_path);
                if let Ok(model) = crate::semantic::TfidfModel::load(&model_path) {
                    let query_vec = crate::semantic::compute_query_vector(
                        &args.query,
                        &model.vocab,
                        model.doc_count,
                        &model.df,
                    );
                    let sem_results = emb_store.search(&query_vec, lim);
                    let sem_weight = args.semantic_weight.unwrap_or(0.5);

                    if semantic_only {
                        // Replace items entirely with semantic results
                        items.clear();
                        if let Ok(store) =
                            self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                        {
                            for (sym_id, score) in &sem_results {
                                if let Ok(Some(rec)) = store.symbol_record(sym_id) {
                                    items.push(json!({
                                        "ref": {
                                            "id": rec.id,
                                            "stable_id": rec.id,
                                            "source_id": args.source_id,
                                            "domain": "code",
                                            "kind": "symbol",
                                            "path": rec.file_path,
                                            "title": rec.id.rsplit("::").next().unwrap_or("").split('#').next().unwrap_or(""),
                                            "summary": rec.stored_source.lines().next().unwrap_or(""),
                                        },
                                        "snippet": rec.stored_source.lines().next().unwrap_or(""),
                                        "score": score,
                                        "line": rec.start_line,
                                    }));
                                }
                            }
                        }
                    } else if sem_weight > 0.0 {
                        // Hybrid: merge semantic results with BM25
                        let existing_ids: HashSet<String> = items
                            .iter()
                            .filter_map(|i| i["ref"]["id"].as_str().map(|s| s.to_string()))
                            .collect();

                        if let Ok(store) =
                            self.ensure_repo_store(&args.source_id, args.repo_path.as_deref())
                        {
                            for (sym_id, score) in &sem_results {
                                if existing_ids.contains(sym_id) {
                                    continue;
                                }
                                if let Ok(Some(rec)) = store.symbol_record(sym_id) {
                                    items.push(json!({
                                        "ref": {
                                            "id": rec.id,
                                            "stable_id": rec.id,
                                            "source_id": args.source_id,
                                            "domain": "code",
                                            "kind": "symbol",
                                            "path": rec.file_path,
                                            "title": rec.id.rsplit("::").next().unwrap_or("").split('#').next().unwrap_or(""),
                                            "summary": rec.stored_source.lines().next().unwrap_or(""),
                                        },
                                        "snippet": rec.stored_source.lines().next().unwrap_or(""),
                                        "score": score * sem_weight,
                                        "line": rec.start_line,
                                    }));
                                }
                            }
                        }

                        // Re-sort by score
                        items.sort_by(|a, b| {
                            let sa = a["score"].as_f64().unwrap_or(0.0);
                            let sb = b["score"].as_f64().unwrap_or(0.0);
                            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                }
            }
        }

        let mut budget_truncated = false;
        if let Some(mt) = args.max_tokens {
            let (packed, bt) = pack_catalog_items_to_token_budget(items, mt);
            items = packed;
            budget_truncated = bt;
        }

        let mut limit_truncated = items.len() > lim || more_file_hits_exist;
        if items.len() > lim {
            warnings.push(json!({
                "code": "TRUNCATED",
                "message": format!("results truncated to limit {lim}"),
            }));
            items.truncate(lim);
            limit_truncated = true;
        }

        let meta_truncated = budget_truncated || limit_truncated;

        let timer = MetaBuilder::new(raw_bytes.max(1));
        let mut data = json!({ "items": items });
        if limit_truncated && let Value::Object(ref mut map) = data {
            map.insert("truncated".to_string(), json!(true));
        }
        let mut extra: Vec<(String, Value)> =
            vec![("truncated".to_string(), json!(meta_truncated))];
        if semantic_enabled || semantic_only {
            extra.push(("search_mode".to_string(), json!(search_mode)));
        }
        self.success_response_with_meta_extra(&timer, data, warnings, extra)
    }
}
