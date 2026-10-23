# symslice-mcp plugin

Agentpack plugin that registers symslice-mcp as an MCP server and installs
a routing policy that steers agents toward structured retrieval.

## Install

```bash
agentpack add olegpustovit/symslice-mcp/plugins/symslice-mcp
agentpack sync
```

## Requirements

- `symslice-mcp` binary on PATH
- An indexed repository (`symslice-mcp ingest .`)

## What's included

- **mcp.json**: Registers `symslice-mcp serve` as an MCP server
- **rules/symslice-routing.mdc**: Always-on routing policy with full tool table
- **agents/symslice-advisor.md**: Optional agent that helps diagnose usage
