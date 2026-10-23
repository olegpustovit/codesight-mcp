# dowser-mcp plugin

Agentpack plugin that registers dowser-mcp as an MCP server and installs
a routing policy that steers agents toward structured retrieval.

## Install

```bash
agentpack add path/to/plugins/dowser-mcp
agentpack sync
```

## Requirements

- `dowser-mcp` binary on PATH
- An indexed repository (`dowser-mcp ingest .`)

## What's included

- **mcp.json**: Registers `dowser-mcp serve` as an MCP server
- **rules/dowser-routing.mdc**: Always-on routing policy with full tool table
- **agents/dowser-advisor.md**: Optional agent that helps diagnose usage
