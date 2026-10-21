use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use codesight_engine::{config::AppConfig, service::CodeService};
use codesight_mcp::cli::{check, hooks, init};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

mod stdio_io;

#[derive(Copy, Clone, Debug, Default, ValueEnum)]
enum LogFormat {
    #[default]
    Pretty,
    Json,
}

#[derive(Parser)]
#[command(name = "codesight-mcp", about = "MCP server (stdio JSON-RPC)")]
struct Args {
    /// Log output format (stderr only; JSON-RPC stays on stdout).
    #[arg(long, value_enum, default_value_t = LogFormat::Pretty, env = "CODESIGHT_LOG_FORMAT")]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the MCP protocol over stdio (default when no subcommand is given).
    Serve {
        /// Repository to serve. Tools resolve the index from the process
        /// working directory; this flag changes into PATH first, for clients
        /// that do not launch MCP servers from the project root.
        #[arg(long, value_name = "PATH")]
        workspace: Option<PathBuf>,
    },
    /// Operator health check: storage, schema, query packs, project config.
    Check {
        /// Optional workspace path whose `.codesight.toml` and readability are checked.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Instead of the health checks, print the cumulative token + cost
        /// savings recorded in the `_savings.json` ledger.
        #[arg(long)]
        savings: bool,
        /// Run the premium-release gate. Ingests the workspace, probes every
        /// P0 trust invariant (module-tree reachability, strict-ref resolution,
        /// reference provenance, latency budgets) and fails on any regression.
        /// Required before shipping structural-analysis claims.
        #[arg(long = "premium-gate")]
        premium_gate: bool,
    },
    /// Diagnostic: call `ingest_sources` against a local path and print the JSON envelope.
    Ingest {
        /// Local path to index.
        path: PathBuf,
        /// Domain filter: code, docs, or auto.
        #[arg(long, default_value = "auto")]
        domains: String,
        /// Force a full (non-incremental) reindex.
        #[arg(long)]
        full: bool,
    },
    /// Diagnostic: call `search_catalog` against an indexed source and print the JSON envelope.
    Search {
        /// Source id (usually printed by `ingest`).
        source_id: String,
        /// Query string.
        query: String,
        /// Max results.
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Restrict domain: code, docs, or hybrid.
        #[arg(long)]
        domain: Option<String>,
    },
    /// Run the full regression test matrix locally (ROADMAP P3.4).
    /// Today this is the premium-gate path — same probes, clearer name.
    /// Intended as a pre-release smoke test that surfaces every P0/P1
    /// trust invariant in a single command.
    #[command(name = "self-check")]
    SelfCheck {
        /// Optional workspace path. Defaults to the current directory.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Claude Code hook: PreToolUse interceptor for Read/Grep/Glob/Bash.
    #[command(name = "hook-pretooluse")]
    HookPreToolUse,
    /// Claude Code hook: PostToolUse auto-reindex after Edit/Write.
    #[command(name = "hook-posttooluse")]
    HookPostToolUse,
    /// Claude Code hook: PreCompact session snapshot injection.
    #[command(name = "hook-precompact")]
    HookPreCompact,
    /// Generate a commented config template or wire this binary into Claude Code.
    Init {
        /// Write to `./.codesight.toml` (current workspace). Default scope.
        #[arg(long)]
        workspace: bool,
        /// Write/merge a `codesight` entry into `./.mcp.json` so Claude Code
        /// discovers this binary in the current workspace.
        #[arg(long = "claude-code")]
        claude_code: bool,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}

fn main() {
    let args = Args::parse();
    init_tracing(args.log_format);
    let exit_code = match args.command.unwrap_or(Command::Serve { workspace: None }) {
        Command::Serve { workspace } => match run(workspace.as_deref()) {
            Ok(()) => 0,
            Err(error) => {
                tracing::error!(error = %error, "codesight-mcp startup or runtime failed");
                eprintln!("codesight-mcp: {error:#}");
                1
            }
        },
        Command::Check {
            workspace,
            savings,
            premium_gate,
        } => {
            let result = if premium_gate {
                check::run_premium_gate(workspace)
            } else {
                check::run(workspace, savings)
            };
            match result {
                Ok(true) => 0,
                Ok(false) => 1,
                Err(error) => {
                    eprintln!("codesight-mcp check: {error:#}");
                    1
                }
            }
        }
        Command::SelfCheck { workspace } => match check::run_premium_gate(workspace) {
            Ok(true) => 0,
            Ok(false) => 1,
            Err(error) => {
                eprintln!("codesight-mcp self-check: {error:#}");
                1
            }
        },
        Command::Ingest {
            path,
            domains,
            full,
        } => match run_ingest(&path, &domains, full) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("codesight-mcp ingest: {error:#}");
                1
            }
        },
        Command::Search {
            source_id,
            query,
            limit,
            domain,
        } => match run_search(&source_id, &query, limit, domain.as_deref()) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("codesight-mcp search: {error:#}");
                1
            }
        },
        Command::HookPreToolUse => hooks::run_pretooluse(),
        Command::HookPostToolUse => hooks::run_posttooluse(),
        Command::HookPreCompact => hooks::run_precompact(),
        Command::Init {
            workspace,
            claude_code,
            force,
        } => match init::run(workspace, claude_code, force) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("codesight-mcp init: {error:#}");
                1
            }
        },
    };
    std::process::exit(exit_code);
}

fn init_tracing(log_format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("warn,codesight_mcp=info,codesight_storage=info,codesight_extractors=info")
    });
    let stderr = std::io::stderr;
    match log_format {
        LogFormat::Pretty => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(stderr))
                .init();
        }
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json().with_writer(stderr))
                .init();
        }
    }
}

fn run(workspace: Option<&std::path::Path>) -> anyhow::Result<()> {
    if let Some(dir) = workspace {
        std::env::set_current_dir(dir)
            .map_err(|err| anyhow::anyhow!("cannot use --workspace {}: {err}", dir.display()))?;
    }
    tracing::info!(
        version = codesight_engine::config::BUILD_VERSION,
        sha = codesight_engine::config::BUILD_SHA,
        build_time = codesight_engine::config::BUILD_TIME,
        "codesight-mcp starting"
    );
    let config = AppConfig::load();
    let service = CodeService::new(config)?;
    let service = Arc::new(service);
    service.install_arc_handle();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run_rmcp_stdio(service))
}

async fn run_rmcp_stdio(service: Arc<CodeService>) -> anyhow::Result<()> {
    use codesight_engine::config::defaults::SHUTDOWN_DRAIN_SECS;
    use rmcp::ServiceExt;
    use rmcp::transport::stdio_pipes;

    let transport = stdio_pipes();
    #[cfg(debug_assertions)]
    let running = if std::env::var_os("CODESIGHT_TEST_BLOCKING_STDIO").is_some() {
        service
            .clone()
            .serve((
                tokio::io::stdin(),
                stdio_io::ObservedWriter(tokio::io::stdout()),
            ))
            .await
    } else {
        service
            .clone()
            .serve((transport.0, stdio_io::ObservedWriter(transport.1)))
            .await
    };
    #[cfg(not(debug_assertions))]
    let running = service
        .clone()
        .serve((transport.0, stdio_io::ObservedWriter(transport.1)))
        .await;
    let running = running.map_err(|err| anyhow::anyhow!("rmcp stdio serve failed: {err}"))?;
    let cancel = running.cancellation_token();

    tokio::spawn(async move {
        shutdown_signal().await;
        tracing::info!("shutdown signal received, cancelling rmcp service");
        cancel.cancel();
    });

    let quit = running
        .waiting()
        .await
        .map_err(|err| anyhow::anyhow!("rmcp service join error: {err}"))?;
    tracing::info!(reason = ?quit, "rmcp service exited");

    let _ = SHUTDOWN_DRAIN_SECS; // preserved for documentation parity with the old drain budget
    service.config().checkpoint_all_wal();
    tracing::info!("shutdown_complete");
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to install SIGTERM handler, falling back to ctrl_c");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn build_service() -> anyhow::Result<CodeService> {
    CodeService::new(AppConfig::load())
}

fn print_envelope(value: &serde_json::Value) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(value)?;
    println!("{s}");
    Ok(())
}

fn run_ingest(path: &std::path::Path, domains: &str, full: bool) -> anyhow::Result<()> {
    let svc = build_service()?;
    let args = serde_json::json!({
        "path": path.to_string_lossy(),
        "domains": domains,
        "incremental": !full,
    });
    let response = svc.dispatch("ingest_sources", args)?;
    print_envelope(&response)?;
    if response.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        anyhow::bail!("ingest_sources returned ok=false");
    }
    Ok(())
}

fn run_search(
    source_id: &str,
    query: &str,
    limit: usize,
    domain: Option<&str>,
) -> anyhow::Result<()> {
    let svc = build_service()?;
    let mut args = serde_json::json!({
        "source_id": source_id,
        "query": query,
        "limit": limit,
    });
    if let Some(d) = domain {
        args["domain"] = serde_json::json!(d);
    }
    let response = svc.dispatch("search_catalog", args)?;
    print_envelope(&response)?;
    if response.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        anyhow::bail!("search_catalog returned ok=false");
    }
    Ok(())
}
