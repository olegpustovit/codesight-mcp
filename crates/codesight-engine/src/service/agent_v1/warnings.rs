//! Canonical warning codes emitted by `agent_v1` tools.
//!
//! Every warning attached to a tool response goes through this module. Adding
//! a new code here is the only way to introduce a new warning to the public
//! surface — that keeps the taxonomy single-sourced for callers and tests.
//!
//! Test coverage: `tests/routing_drift.rs` asserts that every code referenced
//! in `tools.json` and the routing skill is declared here.

use serde_json::{Value, json};

// ---------- calibration (search) ----------

/// Keyword search returned no item with `match_type == "exact"` and no item
/// scored ≥ 0.7 — the result list is suggestive, not authoritative.
pub const NO_EXACT_MATCH: &str = "NO_EXACT_MATCH";

/// Top-K result scores cluster within 0.05 of each other — the ranker can't
/// distinguish them and the caller should re-shape the query.
pub const AMBIGUOUS_IDENTIFIER: &str = "AMBIGUOUS_IDENTIFIER";

/// Total matches > `limit × 10` — the query is too broad for the limit; either
/// raise the limit or narrow with `scope=`.
pub const OVERBROAD_QUERY: &str = "OVERBROAD_QUERY";

/// NL search fell back to BM25 because the dense lane timed out. Caller got
/// lexical results, not semantic ones.
pub const NL_DEGRADED: &str = "NL_DEGRADED";

// ---------- read / outline ----------

/// Whole-file read should outline first.
pub const OUTLINE_FIRST: &str = "OUTLINE_FIRST";

/// Repeated full-file read suggestion (≥2× per session).
pub const SUGGEST_OUTLINE: &str = "SUGGEST_OUTLINE";

/// Whole-file read returned more than the configured byte threshold.
pub const TOKEN_HEAVY_READ: &str = "TOKEN_HEAVY_READ";

/// At least one ref in a `read` batch errored; remaining refs were processed.
/// Per-ref errors land in `data.errors[]`; successful refs in `data.items[]`.
pub const PARTIAL_BATCH: &str = "PARTIAL_BATCH";

/// `section:` ref could not find the requested heading in the target file.
/// Carried as a per-ref error code, not a top-level warning.
pub const SECTION_NOT_FOUND: &str = "SECTION_NOT_FOUND";

// ---------- limits / truncation ----------

/// Result list was capped by `limit`; more matches exist (`total_matches`
/// reports the unbounded count).
pub const TRUNCATED: &str = "TRUNCATED";

/// Regex search dropped per-file matches because the global limit was reached
/// before that file's matches were exhausted. `_meta.regex_overflow.files`
/// lists affected files.
pub const REGEX_OVERFLOW: &str = "REGEX_OVERFLOW";

// ---------- analysis (dead code, graph) ----------

/// Dead-code finding lands in a path that may host externally-invoked entry
/// points (cli/, bin/, hooks*.rs, main.rs, lib.rs, [[bin]] targets). Caller
/// must verify before acting; the analyzer can't see external dispatch.
pub const MANUAL_REVIEW_REQUIRED: &str = "MANUAL_REVIEW_REQUIRED";

/// `references kind=importers` mixed authoritative explicit-import edges with
/// heuristic same-package symbol-usage edges (Rust `mod x; x::Y`, Python
/// sibling access, Go same-package). More complete than imports alone but may
/// include intra-package references the parser couldn't statically resolve.
pub const GRAPH_USAGE_HEURISTIC: &str = "GRAPH_USAGE_HEURISTIC";

// ---------- freshness ----------

/// Reindex is in flight for a touched file; result may reflect the previous
/// index. Wait one more retrieval call (~100 ms) or query session status.
pub const STALE_DATA_WARNING: &str = "STALE_DATA_WARNING";

// ---------- helpers ----------

/// Build a minimal warning payload `{code, message}`.
pub fn warning(code: &str, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

/// Build a warning with a `suggested_call` (tool + args) so callers can act
/// without parsing the message.
pub fn warning_with_call(
    code: &str,
    message: impl Into<String>,
    suggested_tool: &str,
    suggested_args: Value,
) -> Value {
    json!({
        "code": code,
        "message": message.into(),
        "suggested_call": { "tool": suggested_tool, "args": suggested_args },
    })
}

/// Append a warning to `response.warnings[]`. Creates the array if absent.
/// No-op when `response` is not a JSON object.
pub fn push(response: &mut Value, warning_value: Value) {
    let Value::Object(body) = response else {
        return;
    };
    let warnings = body
        .entry("warnings".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = warnings {
        arr.push(warning_value);
    }
}

/// Every warning code declared by this module. Used by `tests/routing_drift.rs`
/// to verify that no tool emits a code not declared here.
pub const ALL_CODES: &[&str] = &[
    NO_EXACT_MATCH,
    AMBIGUOUS_IDENTIFIER,
    OVERBROAD_QUERY,
    NL_DEGRADED,
    OUTLINE_FIRST,
    SUGGEST_OUTLINE,
    TOKEN_HEAVY_READ,
    PARTIAL_BATCH,
    SECTION_NOT_FOUND,
    TRUNCATED,
    REGEX_OVERFLOW,
    MANUAL_REVIEW_REQUIRED,
    GRAPH_USAGE_HEURISTIC,
    STALE_DATA_WARNING,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_codes_unique() {
        let mut seen = std::collections::HashSet::new();
        for code in ALL_CODES {
            assert!(seen.insert(*code), "duplicate warning code: {code}");
        }
    }

    #[test]
    fn push_creates_warnings_array_if_absent() {
        let mut response = json!({ "ok": true, "data": {} });
        push(&mut response, warning(TRUNCATED, "capped at 50"));
        let arr = response["warnings"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["code"], TRUNCATED);
    }

    #[test]
    fn push_appends_to_existing_warnings() {
        let mut response = json!({ "ok": true, "warnings": [{"code": "PRE_EXISTING"}] });
        push(&mut response, warning(NL_DEGRADED, "fell back"));
        let arr = response["warnings"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["code"], "PRE_EXISTING");
        assert_eq!(arr[1]["code"], NL_DEGRADED);
    }

    #[test]
    fn warning_with_call_carries_suggested_tool() {
        let w = warning_with_call(
            OUTLINE_FIRST,
            "outline first",
            "outline",
            json!({ "view": "file", "path": "src/lib.rs" }),
        );
        assert_eq!(w["code"], OUTLINE_FIRST);
        assert_eq!(w["suggested_call"]["tool"], "outline");
        assert_eq!(w["suggested_call"]["args"]["view"], "file");
    }
}
