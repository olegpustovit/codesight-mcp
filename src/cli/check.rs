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
            codesight_extractors::extractors::engine::catalogs::QueryPackCatalog::for_workspace_root(
                ws,
            )
        }
        None => Ok(codesight_extractors::extractors::engine::catalogs::QueryPackCatalog::builtin()),
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
