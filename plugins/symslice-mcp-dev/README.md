# symslice-mcp-dev

Dev-mode all-in-one agentpack plugin for dogfooding symslice-mcp on the
symslice repo itself. Combines the MCP server, hooks, and routing policy
into a single plugin — and uses `cargo run` instead of a release binary so
you always test the latest source.

## What's different from the production plugins

| Aspect | `symslice-mcp` + `symslice-hooks` | `symslice-mcp-dev` |
|--------|-------------------------------------|---------------------|
| Binary | `symslice-mcp` from PATH | `cargo run --quiet --manifest-path ...` |
| Plugins | Two separate plugins | One combined plugin |
| Use case | End users | Developing symslice itself |

The `cargo run` invocations use `--manifest-path` pointing at this repo's
`Cargo.toml`, so they work from any working directory. The `--quiet` flag
suppresses Cargo's compilation output to keep hook stderr clean.

## Install

```bash
agentpack add plugins/symslice-mcp-dev
agentpack sync
```

## Trade-offs

- **First invocation** after a code change will trigger a recompile (~5s).
  Subsequent calls use the cached binary (instant).
- Hooks run `cargo run` per invocation, so they're slower than the release
  binary. Acceptable for dogfooding; use the production plugins for users.
- The `--manifest-path` is absolute. If you move the repo, update
  `mcp.json` and `hooks/hooks.json`.

## Included

- `mcp.json` — MCP server via `cargo run -- serve`
- `hooks/hooks.json` — PreToolUse/PostToolUse/PreCompact via `cargo run`
- `rules/symslice-routing.mdc` — always-on routing policy
- `rules/symslice-hooks.mdc` — always-on hooks notice
- `agents/symslice-advisor.md` — retrieval diagnosis agent
