# dowser-hooks

Claude Code hooks plugin for dowser-mcp. Intercepts native Grep/Glob/Bash
tool calls and steers toward dowser structured retrieval.

## Install

```bash
agentpack add path/to/plugins/dowser-hooks
agentpack sync
```

## Requirements

- `dowser-mcp` binary on PATH
- No other dependencies (no shell scripts, no Python, no Node)

## Hooks

All hooks call the `dowser-mcp` binary; Claude Code reads the decision
from the JSON the hook prints on stdout.

| Hook | Matcher | Behavior |
|------|---------|----------|
| PreToolUse | Grep | Denies greps over indexable code/docs and suggests the matching `search` call |
| PreToolUse | Glob | Denies and suggests `outline view=repo` or `search` |
| PreToolUse | Bash | Denies `grep`, `rg`, `find`, `cat`, `head`, `tail`, `less`, `more`, `wc`, `awk`, `sed` (outside an allowlist of safe prefixes) and suggests `search`, `outline`, or `read` |
| PreToolUse | Edit\|Write | Advisory only: may add a note when the file scores low on agent readiness |
| PostToolUse | Edit\|Write | Reindexes the edited file |
| PreCompact | * | Injects a session snapshot into the compaction context |

Read is not intercepted: Claude Code requires a native Read before Edit/Write.
