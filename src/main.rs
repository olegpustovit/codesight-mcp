use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use codesight_mcp::{
    cli::{check, init},
    config::AppConfig,
    mcp::run_stdio,
    service::CodeService,
};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

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
    Serve,
    /// Operator health check: storage, schema, query packs, project config.
    Check {
        /// Optional workspace path whose `.codesight.toml` and readability are checked.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Instead of the health checks, print the cumulative token + cost
        /// savings recorded in the `_savings.json` ledger.
        #[arg(long)]
        savings: bool,
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
    /// Generate a commented config template or wire this binary into Claude Code.
    Init {
        /// Write to `$XDG_CONFIG_HOME/codesight/config.toml` (machine-wide).
        #[arg(long, conflicts_with_all = ["workspace", "claude_code"])]
        global: bool,
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
    let exit_code = match args.command.unwrap_or(Command::Serve) {
        Command::Serve => match run() {
            Ok(()) => 0,
            Err(error) => {
                tracing::error!(error = %error, "codesight-mcp startup or runtime failed");
                eprintln!("codesight-mcp: {error:#}");
                1
            }
        },
        Command::Check { workspace, savings } => match check::run(workspace, savings) {
            Ok(true) => 0,
            Ok(false) => 1,
            Err(error) => {
                eprintln!("codesight-mcp check: {error:#}");
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
        Command::Init {
            global,
            workspace,
            claude_code,
            force,
        } => match init::run(global, workspace, claude_code, force) {
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

fn run() -> anyhow::Result<()> {
    let config = AppConfig::load();
    let service = Arc::new(CodeService::new(config)?);
    run_stdio(service)
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
