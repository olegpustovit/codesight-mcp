mod repo_health;

use serde_json::{Value, json};

use crate::analysis::{dead_code, quality};
use crate::service::CodeService;
use crate::service::agent_v1::args::EvaluateQualityArgs;
use crate::service::agent_v1::validate::ValidatedArgs;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

impl CodeService {
    pub(in crate::service) fn evaluate_quality(&self, v: Value) -> anyhow::Result<Value> {
        let args: EvaluateQualityArgs = ValidatedArgs::from_validated(v)?.into_inner();
        let a = args.analysis.to_ascii_lowercase();
        let timer = MetaBuilder::new(0);

        match a.as_str() {
            "complexity" => self.quality_complexity(&args, &timer),
            "repo_health" => self.quality_repo_health(&args, &timer),
            "dead_code" => self.quality_dead_code(&args, &timer),
            "dead_code_v2" => self.quality_dead_code_v2(&args, &timer),
            tail => self.evaluate_quality_tail(&args, tail, &timer),
        }
    }

    fn quality_complexity(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let Some(r) = args.refs.as_ref().and_then(|x| x.first()) else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "complexity requires refs[0].".to_string(),
                    detail: json!({}),
                },
            );
        };
        let Some(id) = r
            .stable_id
            .clone()
            .or(r.id.clone())
            .filter(|_| r.kind.eq_ignore_ascii_case("symbol"))
        else {
            return self.error_response(
                timer,
                ToolError {
                    category: ToolErrorCategory::InvalidArgument,
                    code: "INVALID_REQUEST",
                    message: "complexity requires a symbol ref.".to_string(),
                    detail: json!({}),
                },
            );
        };
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };
        let data = match quality::complexity_payload(&store, &id)? {
            Some(data) => data,
            None => {
                return self.error_response(
                    timer,
                    ToolError {
                        category: ToolErrorCategory::NotFound,
                        code: "NOT_FOUND",
                        message: format!("Symbol '{id}' not found."),
                        detail: json!({ "id": id }),
                    },
                );
            }
        };
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        self.success_response(&timer, data)
    }

    fn quality_dead_code(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };
        let granularity = args.granularity.as_deref().unwrap_or("file");
        let min_confidence = args.min_confidence.unwrap_or(0.5);
        let include_tests = args.include_tests.unwrap_or(true);
        let entry_patterns = args.entry_point_patterns.clone().unwrap_or_default();
        let limit = args.top_n.unwrap_or(25);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let mut data = dead_code::dead_code_payload(
            &store,
            granularity,
            min_confidence,
            include_tests,
            &entry_patterns,
            Some(limit),
        )?;
        if let Some(budget) = args.max_tokens {
            dead_code::pack_dead_code_to_budget(&mut data, budget);
        }
        self.success_response_with_meta_extra(
            &timer,
            data,
            Vec::new(),
            [
                ("methodology".to_string(), json!("multi_signal")),
                ("confidence_level".to_string(), json!("medium")),
            ],
        )
    }

    fn quality_dead_code_v2(
        &self,
        args: &EvaluateQualityArgs,
        timer: &MetaBuilder,
    ) -> anyhow::Result<Value> {
        let store = match self.ensure_repo_store(&args.source_id, args.repo_path.as_deref()) {
            Ok(s) => s,
            Err(e) => return self.error_response(timer, e),
        };
        let min_confidence = args.min_confidence.unwrap_or(0.5);
        let raw_bytes = store.repo_total_bytes()?;
        let timer = MetaBuilder::new(raw_bytes);
        let data = dead_code::dead_code_v2_payload(&store, min_confidence)?;
        self.success_response(&timer, data)
    }
}
