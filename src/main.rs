use codesight_mcp::{config::AppConfig, mcp::run_stdio, service::CodeService};

fn main() {
    if let Err(error) = run() {
        eprintln!("codesight-mcp: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let config = AppConfig::load();
    let service = CodeService::new(config)?;
    run_stdio(&service)
}
