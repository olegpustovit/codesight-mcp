# codesight-hooks

Claude Code hooks plugin for codesight-mcp. Intercepts native Read/Grep/Glob/Bash
tool calls and steers toward codesight structured retrieval.

## Install

```bash
agentpack add path/to/plugins/codesight-hooks
agentpack sync
```

## Requirements

- `codesight-mcp` binary on PATH
- No other dependencies (no shell scripts, no Python, no Node)

## Hooks

| Hook | Matcher | Behavior |
|------|---------|----------|
| PreToolUse | Read | Warns on large code file reads without offset/limit |
| PreToolUse | Grep | Suggests search_catalog |
| PreToolUse | Glob | Suggests inspect_structure view=file_tree |
| PreToolUse | Bash | Warns on grep/rg/find/cat targeting code files |
| PostToolUse | Edit\|Write | Auto-reindexes the edited file |
| PreCompact | * | Injects session snapshot into compaction context |

## Configuration

| Env var | Default | Effect |
|---------|---------|--------|
| `CODESIGHT_HOOK_MIN_SIZE` | `4096` | Files below this size pass through silently |
| `CODESIGHT_HOOK_BLOCK_READS` | `0` | `1` = deny instead of warn |
| `CODESIGHT_HOOK_ALLOW_EDIT_READS` | `1` | `1` = allow reads with offset/limit |
