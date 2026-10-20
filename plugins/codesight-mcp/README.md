# codesight-mcp plugin

Agentpack plugin that registers codesight-mcp as an MCP server and installs
a routing policy that steers agents toward structured retrieval.

## Install

```bash
agentpack add path/to/plugins/codesight-mcp
agentpack sync
```

## Requirements

- `codesight-mcp` binary on PATH
- An indexed repository (`codesight-mcp ingest .`)

## What's included

- **mcp.json**: Registers `codesight-mcp serve` as an MCP server
- **rules/codesight-routing.mdc**: Always-on routing policy with full tool table
- **agents/codesight-advisor.md**: Optional agent that helps diagnose usage
