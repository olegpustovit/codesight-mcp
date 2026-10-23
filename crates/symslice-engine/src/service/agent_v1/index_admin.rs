//! `manage_indexes` dispatch + pure index-lifecycle actions
//! (status / invalidate / delete / rebuild / analysis_health).
//! Session-lifecycle actions live in `session_admin.rs`.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::{IngestSourcesArgs, ManageIndexesArgs};
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::{Confidence, MetaBuilder};

fn invalid_request(message: String, detail: Value) -> ToolError {
    ToolError {
        category: ToolErrorCategory::InvalidArgument,
        code: "INVALID_REQUEST",
        message,
        detail,
    }
}

fn not_indexed(source_id: &str) -> ToolError {
    ToolError {
        category: ToolErrorCategory::InvalidArgument,
        code: "NOT_INDEXED",
        message: format!("No index for '{source_id}'."),
        detail: json!({ "source_id": source_id }),
    }
}

impl CodeService {
    pub(in crate::service) fn manage_indexes(&self, v: Value) -> anyhow::Result<Value> {
        let args: ManageIndexesArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let action = args.action.to_ascii_lowercase();
        let timer = MetaBuilder::new(0).with_confidence(Confidence::Safe);

        match action.as_str() {
            "invalidate" | "delete" => self.manage_invalidate(&args, &action, &timer),
            "rebuild" => self.manage_rebuild(&args, &timer),
            "status" => self.manage_status(&args, &timer),
            "snapshot_quality" => self.manage_snapshot_quality(&args, &timer),
            "session_stats" => self.manage_session_stats(&timer),
            "session_end" => self.manage_session_end(&timer),
            "analysis_health" => self.manage_analysis_health(&args, &timer),
            _ => self.error_response(
                &timer,
                invalid_request(
                    format!("Unknown manage_indexes action '{}'.", args.action),
                    json!({ "action": args.action }),
                ),
            ),
        }
    }

    /// Surface graph evidence quality, reachability inputs and staleness.
    /// Unresolved specifiers are not retained, so completeness is unmeasured.
    fn manage_analysis_health(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let Some(sid) = args.source_id.as_deref() else {
            return self.error_response(
                timer,
                invalid_request(
                    "source_id required for analysis_health.".to_string(),
                    json!({}),
                ),
            );
        };
        let store = require_store!(repo: self, sid, None, timer);
        let graph = store.dependency_graph()?;
        let graph_with_quality = store.dependency_graph_with_import_quality()?;
        let all_files = store.all_file_paths()?;

        let mut workspace_crate_edges = 0u64;
        let mut reexport_edges = 0u64;
        for targets in graph.values() {
            for target in targets {
                if target.ends_with("/src/lib.rs") && target.starts_with("crates/") {
                    workspace_crate_edges += 1;
                }
                if target.ends_with("/mod.rs") || target.ends_with("/lib.rs") {
                    reexport_edges += 1;
                }
            }
        }

        let total_edges: u64 = graph.values().map(|t| t.len() as u64).sum();

        // Stored candidates measure evidence quality, not unresolved specifiers,
        // which are not retained by the import pipeline.
        let strict_graph = store.dependency_graph_strict()?;
        let heuristic_edges: Vec<_> = graph_with_quality
            .iter()
            .flat_map(|(source, targets)| {
                targets.iter().map(move |(target, q)| (source, target, q))
            })
            .filter(|(_, _, q)| matches!(q, symslice_core::symbols::FactQuality::Heuristic))
            .collect();
        let heuristic_import_edges = heuristic_edges.len();
        let heuristic_edges_in_strict_graph = heuristic_edges
            .iter()
            .filter(|(source, target, _)| {
                strict_graph
                    .get(*source)
                    .is_some_and(|targets| targets.contains(*target))
            })
            .count();

        let roots = all_files
            .iter()
            .filter(|f| {
                let basename = f.rsplit('/').next().unwrap_or(f.as_str());
                symslice_language_support::is_entrypoint_basename(basename)
            })
            .count() as u64;

        let in_flight_repos = self.reindex_states().in_flight_repos();
        let in_flight = in_flight_repos.iter().any(|repo| repo == sid);

        self.success_response(
            timer,
            json!({
                "action": "analysis_health",
                "source_id": sid,
                "reachability": {
                    "roots": roots,
                    "indexed_files": all_files.len(),
                },
                "graph": {
                    "total_edges": total_edges,
                    "workspace_crate_edges": workspace_crate_edges,
                    "reexport_edges": reexport_edges,
                    "heuristic_import_edges": heuristic_import_edges,
                    "heuristic_edges_in_strict_graph": heuristic_edges_in_strict_graph,
                    "unresolved_import_coverage": "not_measured",
                },
                "staleness": {
                    "in_flight": in_flight,
                },
            }),
        )
    }

    fn manage_invalidate(
        &self,
        args: &ManageIndexesArgs,
        action: &str,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request(
                        "source_id required for invalidate/delete.".to_string(),
                        json!({}),
                    ),
                );
            }
        };
        let had_any = self.config.repo_dir(sid).exists() || self.config.doc_dir(sid).exists();
        if !had_any {
            return self.error_response(timer, not_indexed(sid));
        }
        self.wipe_indexes_for_repo(sid)?;
        self.source_registry().invalidate(sid);
        self.bump_catalog_generation();
        self.success_response(
            timer,
            json!({
                "action": action,
                "source_id": sid,
                "message": "Indexes removed for source.",
            }),
        )
    }

    fn manage_rebuild(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let root = match args.root_hint.as_deref() {
            Some(r) => r,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("root_hint required for rebuild.".to_string(), json!({})),
                );
            }
        };
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("source_id required for rebuild.".to_string(), json!({})),
                );
            }
        };
        self.wipe_indexes_for_repo(sid)?;
        self.source_registry().invalidate(sid);
        let ingest = IngestSourcesArgs {
            input_kind: "local_path".to_string(),
            domains: args.domains.clone().unwrap_or_else(|| "auto".to_string()),
            path: root.to_string(),
            incremental: false,
            enrichment: vec![],
        };
        self.ingest_sources(serde_json::to_value(ingest)?)
    }

    fn manage_status(
        &self,
        args: &ManageIndexesArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let sid = match args.source_id.as_deref() {
            Some(s) => s,
            None => {
                return self.error_response(
                    timer,
                    invalid_request("source_id required for status.".to_string(), json!({})),
                );
            }
        };
        if let Ok(store) = self.ensure_repo_store(sid, None) {
            let meta = store.repo_metadata()?;
            // ROADMAP P1.1: manage_indexes is metadata — null baseline.
            let timer_b = MetaBuilder::new(0).with_confidence(Confidence::Safe);
            return self.success_response(
                &timer_b,
                json!({
                    "action": "status",
                    "source_id": sid,
                    "code_index": serde_json::to_value(&meta)?,
                }),
            );
        }
        if let Ok(dstore) = self.ensure_doc_store(sid, None) {
            let timer_b = MetaBuilder::new(0);
            let toc = dstore.toc().map(|t| t.len()).unwrap_or(0);
            return self.success_response(
                &timer_b,
                json!({
                    "action": "status",
                    "source_id": sid,
                    "doc_index": { "sections_indexed": toc },
                }),
            );
        }
        self.error_response(timer, not_indexed(sid))
    }
}
