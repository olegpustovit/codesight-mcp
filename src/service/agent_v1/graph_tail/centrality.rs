//! `inspect_graph graph=centrality` — pagerank or degree-based node ranking.

use serde_json::{Value, json};

use crate::service::CodeService;
use crate::service::agent_v1::args::InspectGraphArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::require_store;
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(super) fn graph_centrality(&self, args: &InspectGraphArgs) -> anyhow::Result<Value> {
        let store = require_store!(repo: self, &args.source_id, args.repo_path.as_deref(), &MetaBuilder::new(0));
        let algo = args.algorithm.as_deref().unwrap_or("pagerank");
        if algo != "pagerank" && algo != "degree" {
            return self.error_response(
                &MetaBuilder::new(0),
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: format!(
                        "Unknown centrality algorithm '{algo}'. Use 'pagerank' or 'degree'."
                    ),
                    detail: json!({ "algorithm": algo }),
                },
            );
        }

        let adj = store.dependency_graph()?;
        let degrees = crate::analysis::graph::compute_in_out_degrees(&adj);
        let top_n = args.top_n.unwrap_or(20);

        let (ranked, iterations) = if algo == "pagerank" {
            let pr = crate::analysis::graph::compute_pagerank(&adj, 0.85, 100, 1e-6);
            let mut ranked: Vec<_> = pr.scores.iter().collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
            let ranked: Vec<Value> = ranked
                .iter()
                .take(top_n)
                .enumerate()
                .map(|(i, (node, score))| {
                    let deg = degrees.get(*node);
                    json!({
                        "path": node,
                        "rank": i + 1,
                        "score": score,
                        "in_degree": deg.map(|d| d.in_degree).unwrap_or(0),
                        "out_degree": deg.map(|d| d.out_degree).unwrap_or(0),
                        "kind": "file",
                    })
                })
                .collect();
            (ranked, pr.iterations)
        } else {
            let mut ranked: Vec<_> = degrees.iter().collect();
            ranked.sort_by(|a, b| {
                (b.1.in_degree + b.1.out_degree).cmp(&(a.1.in_degree + a.1.out_degree))
            });
            let ranked: Vec<Value> = ranked
                .iter()
                .take(top_n)
                .enumerate()
                .map(|(i, (node, deg))| {
                    json!({
                        "path": node,
                        "rank": i + 1,
                        "score": (deg.in_degree + deg.out_degree) as f64,
                        "in_degree": deg.in_degree,
                        "out_degree": deg.out_degree,
                        "kind": "file",
                    })
                })
                .collect();
            (ranked, 0)
        };

        self.success_response(
            &MetaBuilder::new(0),
            json!({
                "graph": "centrality",
                "algorithm": algo,
                "ranked_files": ranked,
                "iterations_to_converge": iterations,
            }),
        )
    }
}
