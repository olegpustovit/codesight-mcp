//! `search_catalog` — BM25 symbols/files/sections with optional fuzzy + semantic layers.

mod dense;
mod files;
mod fuzzy;
mod plan;
mod response;
mod semantic;

use serde_json::{Value, json};

use crate::service::agent_v1::args::SearchCatalogArgs;
use crate::service::agent_v1::util::{search_hit_section, search_hit_symbol};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory, index_error};
use crate::service::savings::MetaBuilder;
use crate::service::{CodeService, MAX_QUERY_LEN};
use codesight_storage::storage::IndexStore;
use codesight_storage::storage::doc_index::DocStore;

use dense::{
    materialize_dense_section_items, materialize_dense_symbol_items, merge_dense_ranked_items,
};
use plan::{
    CodeSideStats, KindFlags, LexicalStats, SearchCounts, SearchPlan, build_meta_extras,
    estimate_candidate_file_bytes,
};
use response::{FormatCtx, format_output};
use semantic::{run_dense_lane, semantic_fallback_warning};

impl CodeService {
    fn resolve_stores(
        &self,
        args: &SearchCatalogArgs,
        domain: &str,
        flags: &KindFlags,
        warnings: &mut Vec<Value>,
    ) -> Result<(Option<IndexStore>, Option<DocStore>), ToolError> {
        let need_code = domain != "docs" && (flags.want_symbol || flags.want_file);
        let need_doc =
            domain != "code" && (flags.want_section || domain == "docs" || domain == "hybrid");

        let code_store = if need_code {
            match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" => {
                    warnings.push(json!({
                        "code": "CODE_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let doc_store = if need_doc {
            match self.ensure_doc_store(&args.source_id, args.repo_path.as_deref()) {
                Ok(s) => Some(s),
                Err(e) if domain == "hybrid" && code_store.is_some() => {
                    warnings.push(json!({
                        "code": "DOC_INDEX_MISSING",
                        "message": e.message,
                    }));
                    None
                }
                Err(e) if domain == "docs" || domain == "hybrid" => return Err(e),
                Err(_) => None,
            }
        } else {
            None
        };

        if code_store.is_none() && doc_store.is_none() {
            return Err(ToolError {
                category: ToolErrorCategory::InvalidArgument,
                code: "NOT_INDEXED",
                message: format!(
                    "Source '{}' has no usable index for this search.",
                    args.source_id
                ),
                detail: json!({ "source_id": args.source_id, "domain": domain }),
            });
        }
        Ok((code_store, doc_store))
    }

    #[allow(clippy::too_many_arguments)]
    fn search_code_side(
        &self,
        store: &IndexStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        flags: &KindFlags,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<CodeSideStats, ToolError> {
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
        let want_body = args.include_body.unwrap_or(false);
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let mut more_file_hits = false;
        let mut fuzzy_applied = false;
        if flags.want_file {
            let qmode = args.query_mode.as_deref().unwrap_or("keyword");
            let result = self
                .search_code_files_text(
                    store,
                    &args.source_id,
                    &args.query,
                    scope.cloned(),
                    exclude,
                    per_kind_cap,
                    qmode,
                    ctx_lines,
                )
                .map_err(index_error)?;
            counts.add(result.total_matches, result.files_with_matches);
            more_file_hits = result.more_file_hits;
            items.extend(result.items);
        }
        if flags.want_symbol || (!flags.want_file && !flags.want_section) {
            let mut symbols = self.run_search(
                store,
                args.query.clone(),
                scope.cloned(),
                exclude,
                args.kind.clone(),
                per_kind_cap,
            )?;
            let fuzzy_explicit = args.fuzzy == Some(true);
            let auto_fuzzy =
                !fuzzy_explicit && symbols.is_empty() && args.fuzzy_threshold.is_some();
            if fuzzy_explicit && !symbols.is_empty() {
                for s in &mut symbols {
                    s.match_type = Some("exact".to_string());
                }
            }
            if fuzzy_explicit || auto_fuzzy {
                fuzzy_applied = fuzzy::augment_with_fuzzy(
                    store,
                    args,
                    scope.map(String::as_str),
                    exclude,
                    per_kind_cap,
                    auto_fuzzy,
                    &mut symbols,
                ) > 0;
            }
            for s in symbols {
                let body = if want_body {
                    self.symbol_body_with_context(store, &s.id, ctx_lines)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                items.push(search_hit_symbol(&args.source_id, &s, body.as_deref()));
            }
        }
        Ok(CodeSideStats {
            more_file_hits,
            fuzzy_applied,
        })
    }

    fn search_doc_side(
        &self,
        store: &DocStore,
        args: &SearchCatalogArgs,
        scope: Option<&String>,
        per_kind_cap: usize,
        items: &mut Vec<Value>,
    ) -> Result<(), ToolError> {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        let want_body = args.include_body.unwrap_or(false);
        let ctx_lines = args.context_lines.unwrap_or(0).min(50);
        let trimmed = args.query.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let effective_query = if trimmed.len() > MAX_QUERY_LEN {
            &trimmed[..MAX_QUERY_LEN]
        } else {
            trimmed
        };
        let sections = store
            .search_sections(
                effective_query,
                scope.map(String::as_str),
                exclude,
                per_kind_cap,
            )
            .map_err(index_error)?;
        for sec in sections {
            let body = if want_body {
                self.section_body_with_context(store, &sec.id, ctx_lines)
                    .ok()
                    .flatten()
            } else {
                None
            };
            items.push(search_hit_section(&args.source_id, &sec, body.as_deref()));
        }
        Ok(())
    }

    pub(in crate::service) fn search_catalog(&self, v: Value) -> anyhow::Result<Value> {
        let mut args: SearchCatalogArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let plan = match SearchPlan::build(&mut args) {
            Ok(p) => p,
            Err(e) => return self.error_response(&MetaBuilder::new(0), e),
        };

        let mut warnings = Vec::new();
        let (code_store, doc_store) =
            match self.resolve_stores(&args, &plan.domain, &plan.flags, &mut warnings) {
                Ok(pair) => pair,
                Err(e) => return self.error_response(&MetaBuilder::new(0), e),
            };

        let mut items: Vec<Value> = Vec::new();
        let mut counts = SearchCounts::default();
        self.run_count_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut counts,
        );

        let stats = match self.run_lexical_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut counts,
        ) {
            Ok(s) => s,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        let semantic_requested = match self.run_semantic_pass(
            &args,
            &plan,
            code_store.as_ref(),
            doc_store.as_ref(),
            &mut items,
            &mut warnings,
        ) {
            Ok(b) => b,
            Err(e) => return self.error_response(&MetaBuilder::new(1), e),
        };

        let raw_bytes =
            match estimate_candidate_file_bytes(code_store.as_ref(), doc_store.as_ref(), &items) {
                Ok(bytes) => bytes,
                Err(e) => return self.error_response(&MetaBuilder::new(1), e),
            };
        let timer = MetaBuilder::new(raw_bytes.max(1))
            .with_class(crate::service::savings::RetrievalClass::Search);
        let extra = build_meta_extras(&plan, stats.fuzzy_applied, semantic_requested);

        let fmt_ctx = FormatCtx {
            output_mode: &plan.output_mode,
            query: &args.query,
            query_mode_effective: &plan.query_mode_effective,
            offset: plan.offset,
            lim: plan.lim,
            max_tokens: args.max_tokens,
            more_file_hits_exist: stats.more_file_hits_exist,
            counts,
        };
        format_output(self, &fmt_ctx, items, &timer, warnings, extra)
    }

    fn run_count_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        counts: &mut SearchCounts,
    ) {
        let exclude = args.exclude.as_deref().unwrap_or(&[]);
        if let Some(store) = code_store
            && (plan.flags.want_symbol
                || (!plan.flags.want_file && !plan.flags.want_section))
            && let Ok((m, f)) = store.count_symbols(
                &args.query,
                args.kind.as_deref(),
                plan.scope.as_deref(),
                exclude,
            )
        {
            counts.add(m, f);
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
            && let Ok((m, f)) = dstore.count_sections(&args.query, plan.scope.as_deref(), exclude)
        {
            counts.add(m, f);
        }
    }

    fn run_lexical_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        counts: &mut SearchCounts,
    ) -> Result<LexicalStats, ToolError> {
        let mut more_file_hits_exist = false;
        let mut fuzzy_applied = false;
        if let Some(store) = code_store {
            let stats = self.search_code_side(
                store,
                args,
                plan.scope.as_ref(),
                &plan.flags,
                plan.per_kind_cap,
                items,
                counts,
            )?;
            more_file_hits_exist = stats.more_file_hits;
            fuzzy_applied = stats.fuzzy_applied;
        }
        if let Some(dstore) = doc_store
            && (plan.flags.want_section || plan.domain == "docs" || plan.domain == "hybrid")
        {
            self.search_doc_side(dstore, args, plan.scope.as_ref(), plan.per_kind_cap, items)?;
        }
        Ok(LexicalStats {
            more_file_hits_exist,
            fuzzy_applied,
        })
    }

    fn run_semantic_pass(
        &self,
        args: &SearchCatalogArgs,
        plan: &SearchPlan,
        code_store: Option<&IndexStore>,
        doc_store: Option<&DocStore>,
        items: &mut Vec<Value>,
        warnings: &mut Vec<Value>,
    ) -> Result<bool, ToolError> {
        let semantic_requested =
            args.semantic.unwrap_or(false) || plan.query_mode_effective == "natural_language";
        if !semantic_requested {
            return Ok(false);
        }
        if !(plan.flags.want_symbol || plan.flags.want_section) {
            warnings.push(semantic_fallback_warning(
                "files_only",
                "request only targets lexical file hits".to_string(),
            ));
            return Ok(true);
        }
        let dense = run_dense_lane(self, &args.query, code_store, doc_store, plan.lim);
        if let Some(w) = dense.warning {
            warnings.push(w);
            return Ok(true);
        }
        let include_body = args.include_body.unwrap_or(false);
        let context_lines = args.context_lines.unwrap_or(0).min(50);
        let dense_symbol_items = materialize_dense_symbol_items(
            self,
            &args.source_id,
            code_store,
            &dense.symbol_hits,
            include_body,
            context_lines,
        )?;
        let dense_section_items = materialize_dense_section_items(
            self,
            &args.source_id,
            doc_store,
            &dense.section_hits,
            include_body,
            context_lines,
        )?;
        let taken = std::mem::take(items);
        *items = merge_dense_ranked_items(
            taken,
            &dense.symbol_hits,
            &dense.section_hits,
            dense_symbol_items,
            dense_section_items,
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semantic::DenseRankedHit;

    fn symbol_item(id: &str) -> Value {
        json!({
            "ref": { "id": id, "kind": "symbol" },
            "snippet": "sig",
        })
    }

    fn file_item(path: &str) -> Value {
        json!({
            "ref": { "path": path, "kind": "file" },
            "snippet": "body",
        })
    }

    #[test]
    fn dense_only_hits_are_materialized_and_files_are_appended() {
        let merged = merge_dense_ranked_items(
            vec![file_item("src/main.rs"), symbol_item("lex")],
            &[
                DenseRankedHit {
                    id: "dense".to_string(),
                    distance: 0.1,
                },
                DenseRankedHit {
                    id: "lex".to_string(),
                    distance: 0.2,
                },
            ],
            &[],
            std::collections::HashMap::from([("dense".to_string(), symbol_item("dense"))]),
            std::collections::HashMap::new(),
        );
        let ids: Vec<&str> = merged
            .iter()
            .filter_map(|item| item.pointer("/ref/id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["lex", "dense"]);
        assert_eq!(merged.last().unwrap()["ref"]["kind"], "file");
    }
}
