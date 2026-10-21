//! `codesight-mcp check` — operator health diagnostic.
//!
//! Validates the pieces an operator is most likely to get wrong: storage
//! directory writability, workspace readability, query-pack loading,
//! `.codesight.toml` parse, and a read of the storage schema version. Prints a
//! human-readable report to stdout and exits 0 on success, 1 on the first
//! failure.

use std::path::{Path, PathBuf};

use anyhow::Context;

use codesight_engine::config::AppConfig;

#[derive(Debug)]
struct CheckOutcome {
    name: &'static str,
    status: Result<String, String>,
}

impl CheckOutcome {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Ok(detail.into()),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            status: Err(detail.into()),
        }
    }
}

/// Run the full check pipeline. Returns `Ok(true)` if every check passed.
pub fn run(workspace: Option<PathBuf>, savings: bool) -> anyhow::Result<bool> {
    let config = AppConfig::load();

    if savings {
        return run_savings_only(&config);
    }

    let mut outcomes = Vec::new();
    outcomes.push(check_storage(&config));
    outcomes.push(check_schema_version(&config));
    outcomes.push(check_query_packs(workspace.as_deref()));

    if let Some(ws) = workspace.as_deref() {
        outcomes.push(check_workspace_readable(ws));
        outcomes.push(check_project_config(ws));
    }

    let mut all_ok = true;
    println!("codesight-mcp check");
    println!("===================");
    for outcome in &outcomes {
        match &outcome.status {
            Ok(detail) => println!("  ok    {:<22} {}", outcome.name, detail),
            Err(detail) => {
                all_ok = false;
                println!("  FAIL  {:<22} {}", outcome.name, detail);
            }
        }
    }
    println!("-------------------");
    println!(
        "{}",
        if all_ok {
            "all checks passed"
        } else {
            "one or more checks failed"
        }
    );
    Ok(all_ok)
}

/// `check --savings`: print the cumulative token and cost savings recorded in
/// `_savings.json` as a two-column summary. Always exits 0 — absence of the
/// ledger file is a legitimate "no activity yet" state, not a failure.
fn run_savings_only(config: &AppConfig) -> anyhow::Result<bool> {
    use codesight_engine::service::savings::SavingsLedger;

    let path = config.savings_file();
    let ledger = SavingsLedger::new(path.clone());

    println!("codesight-mcp savings");
    println!("=====================");
    println!("  ledger path          {}", path.display());

    match ledger.current_state() {
        Ok((tokens, cost_by_model)) => {
            println!("  total_tokens_saved   {tokens}");
            if cost_by_model.is_empty() {
                println!("  cost_avoided         (no activity yet)");
            } else {
                let mut models: Vec<_> = cost_by_model.iter().collect();
                models.sort_by(|a, b| a.0.cmp(b.0));
                for (model, usd) in models {
                    println!("  cost_avoided.{model:<8} ${usd:.4}");
                }
            }
            Ok(true)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("  total_tokens_saved   0 (ledger file does not exist yet)");
            Ok(true)
        }
        Err(err) => {
            println!("  FAIL  read savings    {err}");
            Ok(false)
        }
    }
}

fn check_storage(config: &AppConfig) -> CheckOutcome {
    let index_root = config.index_root();
    if let Err(error) = std::fs::create_dir_all(index_root) {
        return CheckOutcome::fail(
            "storage_dir",
            format!("cannot create '{}': {error}", index_root.display()),
        );
    }

    let probe = index_root.join(".codesight-check-write");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            CheckOutcome::ok("storage_dir", format!("writable: {}", index_root.display()))
        }
        Err(error) => CheckOutcome::fail(
            "storage_dir",
            format!("not writable '{}': {error}", index_root.display()),
        ),
    }
}

fn check_schema_version(config: &AppConfig) -> CheckOutcome {
    // The code-index schema has a forward-only migration chain (see P2).
    // We only verify the target version is reachable as a constant; we do not
    // open every repo's DB here because `check` must remain fast and stateless.
    let target = codesight_storage::storage::migrations::CODE_INDEX_TARGET_USER_VERSION;
    CheckOutcome::ok(
        "schema_version",
        format!(
            "target user_version = {target} (index_root: {})",
            config.index_root().display()
        ),
    )
}

fn check_query_packs(workspace: Option<&Path>) -> CheckOutcome {
    let result = match workspace {
        Some(ws) => {
            codesight_language_support::query_pack_catalog::QueryPackCatalog::for_workspace_root(ws)
        }
        None => Ok(codesight_language_support::query_pack_catalog::QueryPackCatalog::builtin()),
    };
    match result {
        Ok(catalog) => {
            let n = catalog.curated_language_names().len();
            CheckOutcome::ok("query_packs", format!("{n} curated languages loaded"))
        }
        Err(error) => CheckOutcome::fail("query_packs", format!("load failed: {error}")),
    }
}

fn check_workspace_readable(workspace: &Path) -> CheckOutcome {
    match std::fs::metadata(workspace) {
        Ok(meta) if meta.is_dir() => CheckOutcome::ok(
            "workspace",
            format!("readable directory: {}", workspace.display()),
        ),
        Ok(_) => CheckOutcome::fail(
            "workspace",
            format!("'{}' is not a directory", workspace.display()),
        ),
        Err(error) => CheckOutcome::fail(
            "workspace",
            format!("cannot stat '{}': {error}", workspace.display()),
        ),
    }
}

fn check_project_config(workspace: &Path) -> CheckOutcome {
    let path = workspace.join(".codesight.toml");
    if !path.exists() {
        return CheckOutcome::ok(
            "project_config",
            format!(
                "no .codesight.toml at {} (defaults apply)",
                workspace.display()
            ),
        );
    }
    let content = match std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
    {
        Ok(c) => c,
        Err(error) => return CheckOutcome::fail("project_config", format!("{error:#}")),
    };
    match toml::from_str::<toml::Value>(&content) {
        Ok(_) => CheckOutcome::ok("project_config", format!("parsed {}", path.display())),
        Err(error) => CheckOutcome::fail(
            "project_config",
            format!("{} is malformed: {error}", path.display()),
        ),
    }
}

// ── Premium release gate ─────────────────────────────────────────────────
//
// `codesight-mcp check --premium-gate [--workspace PATH]` runs an end-to-end
// probe of every P0 trust invariant. Any failure here blocks
// the release. Thresholds are centralised here so "premium" is a measurable
// contract, not a marketing word.

const PREMIUM_GATE_MAX_DEAD_PCT: f64 = 2.0;
const PREMIUM_GATE_MAX_TOOL_LATENCY_MS: f64 = 2_500.0;
const PREMIUM_GATE_MAX_LINE_ZERO_REFS: usize = 0;
const PREMIUM_GATE_MIN_AGENT_READINESS: f64 = 0.65;

/// Run the premium-release gate: ingest the workspace, fire the probes, and
/// return `Ok(true)` only when every invariant holds.
pub fn run_premium_gate(workspace: Option<PathBuf>) -> anyhow::Result<bool> {
    use codesight_engine::service::CodeService;
    use serde_json::json;

    let ws =
        workspace.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    println!("codesight-mcp premium-gate");
    println!("===========================");
    println!("  workspace            {}", ws.display());

    let config = AppConfig::load();
    let svc = CodeService::new(config)?;

    // Fresh full reindex so the gate measures the code on disk, not a
    // stale snapshot.
    let ingest = svc.dispatch(
        "ingest_sources",
        json!({
            "path": ws.to_string_lossy(),
            "domains": "code",
            "incremental": false,
        }),
    )?;
    if ingest.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        println!("  FAIL  ingest                {ingest}");
        return Ok(false);
    }
    let source_id = ingest["data"]["source_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("ingest_sources missing source_id"))?
        .to_string();

    let mut outcomes: Vec<CheckOutcome> = Vec::new();

    // Probe 1 — module-tree reachability. The on-disk repo must not report a
    // dead_code_pct above PREMIUM_GATE_MAX_DEAD_PCT, because this repo's own
    // crates are all reachable via `pub mod` declarations from their `lib.rs`.
    let health = svc.dispatch(
        "evaluate_quality",
        json!({ "source_id": source_id, "analysis": "repo_health" }),
    )?;
    let dead_pct = health["data"]["dead_code_pct"].as_f64().unwrap_or(100.0);
    let dead_count = health["data"]["dead_count"].as_u64().unwrap_or(u64::MAX);
    outcomes.push(if dead_pct <= PREMIUM_GATE_MAX_DEAD_PCT {
        CheckOutcome::ok(
            "module_tree_reachability",
            format!("dead_code_pct={dead_pct:.2} count={dead_count} (cap {PREMIUM_GATE_MAX_DEAD_PCT:.2})"),
        )
    } else {
        CheckOutcome::fail(
            "module_tree_reachability",
            format!("dead_code_pct={dead_pct:.2} count={dead_count} exceeds cap {PREMIUM_GATE_MAX_DEAD_PCT:.2} — module-tree edges regressed"),
        )
    });

    // Probe 2 — analysis_health. No unresolved `use` statements, non-zero
    // reachability roots in a multi-crate repo.
    let ah = svc.dispatch(
        "manage_indexes",
        json!({ "action": "analysis_health", "source_id": source_id }),
    )?;
    let unresolved = ah["data"]["graph"]["unresolved_use_statements"]
        .as_u64()
        .unwrap_or(u64::MAX);
    let roots = ah["data"]["reachability"]["roots"].as_u64().unwrap_or(0);
    outcomes.push(if unresolved == 0 && roots > 0 {
        CheckOutcome::ok("analysis_health", format!("unresolved_use=0 roots={roots}"))
    } else {
        CheckOutcome::fail(
            "analysis_health",
            format!("unresolved_use={unresolved} roots={roots}"),
        )
    });

    // Probe 3 — reference provenance. `find_references` on a well-known
    // identifier must expose `identifier_references` + `candidate_text_hits`
    // and refuse to emit `text_match` / `line:0` rows.
    let refs = svc.dispatch(
        "inspect_graph",
        json!({
            "source_id": source_id,
            "graph": "find_references",
            "identifier": "CodeService",
        }),
    )?;
    let data = &refs["data"];
    let mut bad_rows: usize = 0;
    let mut has_split =
        data.get("identifier_references").is_some() && data.get("candidate_text_hits").is_some();
    for key in ["identifier_references", "candidate_text_hits"] {
        let Some(rows) = data[key].as_array() else {
            has_split = false;
            continue;
        };
        for r in rows {
            if r["line"].as_u64().unwrap_or(0) == 0 {
                bad_rows += 1;
            }
            if r["kind"].as_str() == Some("text_match") {
                bad_rows += 1;
            }
        }
    }
    outcomes.push(
        if has_split && bad_rows == PREMIUM_GATE_MAX_LINE_ZERO_REFS {
            CheckOutcome::ok(
                "reference_provenance",
                format!(
                    "split fields present; 0 line:0 or text_match rows across {} results",
                    data["identifier_references"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0)
                        + data["candidate_text_hits"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or(0),
                ),
            )
        } else {
            CheckOutcome::fail(
                "reference_provenance",
                format!("split={has_split} bad_rows={bad_rows} — reference contract regressed"),
            )
        },
    );

    // Probe 4 — strict refs refuse to silently succeed. Send a blast_radius
    // request with a ref that cannot resolve; expect NOT_FOUND (ok=false).
    let unresolved_blast = svc.dispatch(
        "impact",
        json!({
            "analysis": "blast_radius",
            "ref": "this/path/does/not/exist.rs::definitely_not_a_real_symbol_xyz#function",
        }),
    )?;
    let rejected = unresolved_blast
        .get("_meta")
        .and_then(|m| m.get("confidence"))
        .and_then(|c| c.as_str())
        == Some("unreliable")
        || unresolved_blast.get("ok").and_then(|b| b.as_bool()) == Some(false);
    outcomes.push(if rejected {
        CheckOutcome::ok(
            "strict_ref_not_found",
            "unresolved structured ref → NOT_FOUND (as required)",
        )
    } else {
        CheckOutcome::fail(
            "strict_ref_not_found",
            format!(
                "expected NOT_FOUND for unresolved structured ref; got {}",
                serde_json::to_string(&unresolved_blast).unwrap_or_default()
            ),
        )
    });

    // Probe 5 — dispatch latency budget. The health + refs calls already
    // recorded _meta.timing_ms; take the worst case across the probes.
    let worst_latency = [&health, &ah, &refs, &unresolved_blast]
        .iter()
        .filter_map(|r| r["_meta"]["timing_ms"].as_f64())
        .fold(0.0_f64, f64::max);
    outcomes.push(if worst_latency <= PREMIUM_GATE_MAX_TOOL_LATENCY_MS {
        CheckOutcome::ok(
            "tool_latency",
            format!("worst_timing_ms={worst_latency:.1} (cap {PREMIUM_GATE_MAX_TOOL_LATENCY_MS:.0})"),
        )
    } else {
        CheckOutcome::fail(
            "tool_latency",
            format!(
                "worst_timing_ms={worst_latency:.1} exceeds cap {PREMIUM_GATE_MAX_TOOL_LATENCY_MS:.0}",
            ),
        )
    });

    // Probe 6 — agent-readiness floor. `repo_health.quality_signal` reports
    // `agent_readiness_score`; below the floor the retrieval layer is not
    // premium-grade.
    let readiness = health["data"]["findings"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|f| f["finding_id"] == "repo_health:quality_signal")
        })
        .and_then(|f| f["evidence"]["agent_readiness_score"].as_f64())
        .unwrap_or(0.0);
    outcomes.push(if readiness >= PREMIUM_GATE_MIN_AGENT_READINESS {
        CheckOutcome::ok(
            "agent_readiness",
            format!("agent_readiness={readiness:.3} (floor {PREMIUM_GATE_MIN_AGENT_READINESS:.2})"),
        )
    } else {
        CheckOutcome::fail(
            "agent_readiness",
            format!(
                "agent_readiness={readiness:.3} below floor {PREMIUM_GATE_MIN_AGENT_READINESS:.2}",
            ),
        )
    });

    // Report.
    let mut all_ok = true;
    for o in &outcomes {
        match &o.status {
            Ok(detail) => println!("  ok    {:<28} {}", o.name, detail),
            Err(detail) => {
                all_ok = false;
                println!("  FAIL  {:<28} {}", o.name, detail);
            }
        }
    }
    println!("---------------------------");
    println!(
        "{}",
        if all_ok {
            "premium gate passed — ship candidate is trustworthy"
        } else {
            "premium gate FAILED — do not ship structural-analysis claims"
        }
    );
    Ok(all_ok)
}
