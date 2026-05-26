//! Static dispatch table for `agent-v1` tools.
//!
//! Each row owns the tool's name, its handler, and its timeout class — one
//! source of truth. New tools add one entry here; no parallel string match
//! lives elsewhere.

use serde_json::Value;

use crate::service::CodeService;
use crate::service::helpers::{ToolError, ToolErrorCategory};
use crate::service::savings::MetaBuilder;

type AgentV1Handler = fn(&CodeService, Value) -> anyhow::Result<Value>;

/// Per-tool wall-clock timeout budget. Exceeding it yields a categorized
/// `TIMEOUT` error. The budget shape matches tool cost: targeted reads are
/// fast, analysis rollups take longer, ingest can legitimately run for tens
/// of seconds on a cold repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutClass {
    FastRead,
    Default,
    Heavy,
    Ingest,
}

impl TimeoutClass {
    pub const fn as_ms(self) -> u64 {
        match self {
            Self::FastRead => 3_000,
            Self::Default => 5_000,
            Self::Heavy => 15_000,
            Self::Ingest => 60_000,
        }
    }
}

static AGENT_V1_HANDLERS: &[(&str, AgentV1Handler, TimeoutClass)] = &[
    // CUTPLAN: 6-tool MCP surface. Public adapters live in `cutplan.rs`;
    // legacy handler names below remain wired so internal tests and the
    // adapters themselves can still dispatch by old name during P1→P2.
    ("search", CodeService::cutplan_search, TimeoutClass::Default),
    (
        // Tier 5.3: cold `outline view=repo` runs PageRank over the full
        // dependency graph and partitions the workspace; on large repos
        // that legitimately takes 3-5 s, then the body cache makes
        // subsequent calls < 50 ms. `Default` (5 s) gives the cold path
        // headroom while still keeping warm calls bound by the SLA tier.
        "outline",
        CodeService::cutplan_outline,
        TimeoutClass::Default,
    ),
    ("read", CodeService::cutplan_read, TimeoutClass::FastRead),
    (
        "references",
        CodeService::cutplan_references,
        TimeoutClass::Default,
    ),
    ("impact", CodeService::cutplan_impact, TimeoutClass::Heavy),
    ("health", CodeService::cutplan_health, TimeoutClass::Heavy),
    // ── Legacy handlers — internal-only. Not in the public tools.json.
    //    Adapters above call into these to reuse the existing analysis
    //    backends. They are NOT exposed to MCP clients.
    (
        "ingest_sources",
        CodeService::ingest_sources,
        TimeoutClass::Ingest,
    ),
    (
        "catalog_sources",
        CodeService::catalog_sources,
        TimeoutClass::FastRead,
    ),
    (
        "search_catalog",
        CodeService::search_catalog,
        TimeoutClass::Default,
    ),
    (
        "read_content",
        CodeService::read_content,
        TimeoutClass::FastRead,
    ),
    (
        "inspect_structure",
        CodeService::inspect_structure,
        TimeoutClass::FastRead,
    ),
    (
        "inspect_graph",
        CodeService::inspect_graph,
        TimeoutClass::Default,
    ),
    (
        "evaluate_quality",
        CodeService::evaluate_quality,
        TimeoutClass::Heavy,
    ),
    (
        "manage_indexes",
        CodeService::manage_indexes,
        TimeoutClass::Ingest,
    ),
];

pub(in crate::service) fn dispatch_tool(
    svc: &CodeService,
    tool_name: &str,
    arguments: Value,
) -> anyhow::Result<Option<Value>> {
    for (name, handler, _) in AGENT_V1_HANDLERS {
        if *name == tool_name {
            return Ok(Some(handler(svc, arguments)?));
        }
    }
    Ok(None)
}

/// Look up the timeout budget (milliseconds) for `tool_name`. Unknown tools
/// fall back to [`TimeoutClass::Default`]; they'll still hit the `NOT_FOUND`
/// branch below but won't hang waiting for a budget decision.
pub fn timeout_ms(tool_name: &str) -> u64 {
    for (name, _, class) in AGENT_V1_HANDLERS {
        if *name == tool_name {
            return class.as_ms();
        }
    }
    TimeoutClass::Default.as_ms()
}

pub(in crate::service) fn not_found(svc: &CodeService, tool_name: &str) -> anyhow::Result<Value> {
    svc.error_response(
        &MetaBuilder::new(0),
        ToolError {
            category: ToolErrorCategory::NotFound,
            code: "NOT_FOUND",
            message: format!("Tool '{tool_name}' is not implemented."),
            detail: serde_json::json!({ "tool": tool_name }),
        },
    )
}
