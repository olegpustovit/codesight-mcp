//! `symslice-mcp init` — generate an operator config template.
//!
//! Two scopes:
//! - `--workspace` writes `./.symslice.toml` (applies to the current repo).
//! - `--claude-code` writes `./.mcp.json` with a `symslice` server entry so
//!   Claude Code picks up this binary in the current workspace.
//!
//! Without any flag, defaults to `--workspace`. Refuses to overwrite without
//! `--force`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::{Value, json};

const WORKSPACE_TEMPLATE: &str = r#"# symslice-mcp workspace configuration.
#
# This file is read from the current workspace root. See CONFIGURATION.md for
# every available knob.

# If true, the indexer regenerates the `languages` list below from whatever
# languages it detects in the workspace on each reindex. Set to false to pin
# the list manually.
languages_adaptive = true

# Languages to index in this workspace. When `languages_adaptive = true`, this
# list is rewritten automatically. When false, only the languages listed here
# are extracted.
languages = [
    # "rust",
    # "typescript",
    # "python",
]

# Extra file-extension → language overrides for this workspace. Useful when a
# repository uses non-standard suffixes for a supported language.
# [extra_extensions]
# "mjs" = "javascript"

# Additional directories to search for tree-sitter query packs, in priority
# order. Relative paths are resolved against the workspace root.
# query_pack_roots = [
#     ".symslice/queries",
# ]
"#;

/// Run `symslice-mcp init` with the given flags.
pub fn run(_workspace: bool, claude_code: bool, force: bool) -> anyhow::Result<()> {
    if claude_code {
        let cwd = std::env::current_dir().context("failed to resolve current directory")?;
        let path = write_claude_code_entry(&cwd, force)?;
        println!("wrote {}", path.display());
        print_claude_code_cli_hint();
        return Ok(());
    }

    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let path = cwd.join(".symslice.toml");
    write_template(&path, WORKSPACE_TEMPLATE, force)?;
    println!("wrote {}", path.display());
    Ok(())
}

/// Resolve the current binary path so the generated `.mcp.json` points at
/// *this* build. Falls back to the bare binary name when the current_exe
/// lookup fails (headless sandboxes, etc.) — Claude Code will then resolve it
/// from `$PATH`.
fn current_executable_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "symslice-mcp".to_string())
}

fn build_claude_code_server_entry() -> Value {
    json!({
        "command": current_executable_path(),
        "args": ["serve"],
        "env": {},
    })
}

/// Write or merge a `.mcp.json` with a `symslice` MCP server entry. When the
/// file already exists and `force` is false, the write is refused; when it
/// exists and `force` is true, we merge instead of clobbering so any other
/// servers the user configured survive.
fn write_claude_code_entry(dir: &Path, force: bool) -> anyhow::Result<PathBuf> {
    let path = dir.join(".mcp.json");
    let mut root: Value = if path.exists() {
        if !force {
            bail!(
                "{} already exists; rerun with --force to overwrite the symslice entry",
                path.display()
            );
        }
        let body = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&body)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        json!({})
    };

    let obj = root
        .as_object_mut()
        .context(".mcp.json root must be a JSON object")?;
    let servers = obj
        .entry("mcpServers".to_string())
        .or_insert_with(|| json!({}));
    let servers_obj = servers
        .as_object_mut()
        .context(".mcp.json `mcpServers` must be a JSON object")?;
    servers_obj.insert("symslice".to_string(), build_claude_code_server_entry());

    let pretty = serde_json::to_string_pretty(&root)? + "\n";
    fs::write(&path, pretty).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

fn print_claude_code_cli_hint() {
    let exe = current_executable_path();
    println!("\nEquivalent CLI command (global registration):");
    println!("  claude mcp add symslice {exe}");
}

fn write_template(path: &Path, body: &str, force: bool) -> anyhow::Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; rerun with --force to overwrite",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Test hook: write into an arbitrary directory rather than the current
/// directory or XDG config path. Lets integration tests exercise the same
/// template + overwrite logic the CLI uses.
#[cfg(test)]
pub(crate) fn run_workspace_at(dir: &Path, force: bool) -> anyhow::Result<PathBuf> {
    let path = dir.join(".symslice.toml");
    write_template(&path, WORKSPACE_TEMPLATE, force)?;
    Ok(path)
}

#[cfg(test)]
pub(crate) fn run_claude_code_at(dir: &Path, force: bool) -> anyhow::Result<PathBuf> {
    write_claude_code_entry(dir, force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn workspace_init_writes_template() {
        let dir = tempdir().unwrap();
        let path = run_workspace_at(dir.path(), false).unwrap();
        assert!(path.exists());
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("languages_adaptive"));
        assert!(body.contains("languages ="));
    }

    #[test]
    fn workspace_init_refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        run_workspace_at(dir.path(), false).unwrap();
        let err = run_workspace_at(dir.path(), false).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn workspace_init_overwrites_with_force() {
        let dir = tempdir().unwrap();
        let path = run_workspace_at(dir.path(), false).unwrap();
        fs::write(&path, "stale content").unwrap();
        run_workspace_at(dir.path(), true).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("languages_adaptive"));
    }

    #[test]
    fn claude_code_init_writes_mcp_json() {
        let dir = tempdir().unwrap();
        let path = run_claude_code_at(dir.path(), false).unwrap();
        assert!(path.exists());
        let body = fs::read_to_string(&path).unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        let symslice = &v["mcpServers"]["symslice"];
        assert!(symslice.is_object(), "symslice entry missing: {body}");
        assert_eq!(symslice["args"][0], "serve");
        assert!(symslice["command"].is_string());
    }

    #[test]
    fn claude_code_init_refuses_overwrite_without_force() {
        let dir = tempdir().unwrap();
        run_claude_code_at(dir.path(), false).unwrap();
        let err = run_claude_code_at(dir.path(), false).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn claude_code_init_merges_existing_servers_with_force() {
        let dir = tempdir().unwrap();
        let mcp_path = dir.path().join(".mcp.json");
        fs::write(
            &mcp_path,
            r#"{"mcpServers":{"other":{"command":"other-mcp","args":[]}}}"#,
        )
        .unwrap();

        run_claude_code_at(dir.path(), true).unwrap();

        let body = fs::read_to_string(&mcp_path).unwrap();
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(
            v["mcpServers"]["other"].is_object(),
            "merge dropped 'other' entry: {body}"
        );
        assert!(
            v["mcpServers"]["symslice"].is_object(),
            "merge missing 'symslice': {body}"
        );
    }
}
